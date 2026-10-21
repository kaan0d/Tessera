'use strict';
/* Presets, history, folder watch, update check and the command-line card.
   Shares the globals of app.js (T, $, $$, invoke, icon, s, catalog, setIn, confirmModal, seg). */

let presets = []; // list_presets
let hist = []; // get_history, newest first
let watch = { enabled: false, rules: [], status: [] }; // get_watch
let lastUpdate = null; // check_update result, or {error}
const undoMsg = new Map(); // history id -> [ok, text]
const MAX_RULES = 20;
const KIND_ICON = { image: 'i-image', audio: 'i-audio', video: 'i-film', document: 'i-doc' };
const KIND_ORDER = ['image', 'audio', 'video', 'document'];

/* ---------- small DOM helpers ---------- */
function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function iconBtn(ic, label, cls) {
  const b = el('button', 'iconbtn' + (cls ? ' ' + cls : ''));
  b.type = 'button';
  b.setAttribute('aria-label', label);
  b.title = label;
  b.append(icon(ic));
  return b;
}

function textBtn(ic, label, cls) {
  const b = el('button', 'btn sm' + (cls ? ' ' + cls : ''));
  b.type = 'button';
  if (ic) b.append(icon(ic));
  b.append(el('span', '', label));
  return b;
}

function switchEl(checked, label) {
  const i = el('input', 'switch');
  i.type = 'checkbox';
  i.setAttribute('role', 'switch');
  i.checked = checked;
  i.setAttribute('aria-label', label);
  return i;
}

function option(value, text) {
  const o = el('option', '', text);
  o.value = value;
  return o;
}

function banner(id, msg) {
  $(id).hidden = !msg;
  $(id).textContent = msg ? String(msg) : '';
}

const fileName = (p) => String(p || '').split(/[\\/]/).pop();
const extOf = (p) => (/\.([^.\\/]+)$/.exec(p || '') || [, ''])[1].toLowerCase();
const dirOf = (p) => String(p || '').replace(/[\\/][^\\/]*$/, '');
const kindOfExt = (e) => (catalog && (catalog.sources.find((x) => x.ext === e) || catalog.targets.find((x) => x.ext === e)) || {}).kind || 'document';
const num = (n, d = 1) => new Intl.NumberFormat(LANG, { maximumFractionDigits: d }).format(n);

function fmtBytes(b) {
  return b >= 1e6 ? t('unit.mb', { n: num(b / 1e6) }) : t('unit.kb', { n: num(b / 1e3, 0) });
}

function paintSeg(name) {
  $$(`input[name=${name}]`).forEach((r, i) => r.checked && seg(r.closest('.seg'), i));
}

function paintRange(r, out, unit) {
  r.style.setProperty('--fill', ((r.value - r.min) / (r.max - r.min)) * 100 + '%');
  out.textContent = r.hidden ? '' : r.value + (unit ? ' ' + unit : '');
}

/* ---------- dialogs: focus trap, Escape, focus returns where it was ---------- */
let dlg = null;

function openDlg(id, first) {
  const m = $(id);
  const prev = document.activeElement;
  const key = (e) => {
    if (e.key === 'Escape') return e.preventDefault(), closeDlg();
    if (e.key !== 'Tab') return;
    const f = $$('button,input,select', m).filter((x) => !x.disabled && x.offsetParent);
    const i = f.indexOf(document.activeElement);
    if (e.shiftKey && i <= 0) e.preventDefault(), f[f.length - 1].focus();
    else if (!e.shiftKey && i === f.length - 1) e.preventDefault(), f[0].focus();
  };
  dlg = { m, prev, key };
  document.addEventListener('keydown', key);
  m.hidden = false;
  m.classList.add('open');
  m.querySelector('.dlg-body').scrollTop = 0;
  first.focus();
}

function closeDlg() {
  if (!dlg) return;
  const { m, prev, key } = dlg;
  dlg = null;
  document.removeEventListener('keydown', key);
  m.hidden = true;
  m.classList.remove('open');
  if (prev && prev.isConnected) prev.focus();
}

function msg(id, text) {
  $(id).textContent = text || '';
}

// brings the first problem in a dialog into view
function showFirstError(dlgId) {
  const m = $$(`#${dlgId} .dlg-body .field-msg`).find((x) => x.textContent);
  if (m) m.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
}

async function pickDir() {
  const d = T && T.dialog;
  if (!d) return showError(t('err.no_dialog')), null;
  try {
    const p = await d.open({ directory: true, multiple: false });
    return p ? String(p) : null;
  } catch (e) {
    showError(t('err.pick', { err: e }));
    return null;
  }
}

/* ======================================================================== presets */
function presetSummary(p) {
  const o = p.options || {};
  const parts = [p.target === 'same' ? t('pr.s_same') : p.target.toUpperCase()];
  if (p.video_target && p.kinds.includes('video'))
    parts.push(t('pr.s_video', { fmt: p.video_target === 'same' ? t('pr.s_same_l') : p.video_target.toUpperCase() }));
  const r = o.resize;
  if (r && r.max_edge != null) parts.push(t('pr.s_edge', { n: r.max_edge }));
  else if (r && r.percent != null) parts.push(t('pr.s_pct', { n: r.percent }));
  else if (r && r.exact) parts.push(t('pr.s_exact', { w: r.exact.w, h: r.exact.h, fit: t({ contain: 'fit.contain_l', cover: 'fit.cover_l', stretch: 'fit.stretch_l' }[r.exact.fit] || 'fit.contain_l') }));
  if (o.max_bytes) parts.push(t('pr.s_max', { size: fmtBytes(o.max_bytes) }));
  if (o.jpeg_quality) parts.push(t('pr.s_q', { n: o.jpeg_quality }));
  if (o.video_mbps) parts.push(t('pr.s_v', { n: o.video_mbps }));
  if (o.strip_metadata) parts.push(t('pr.s_strip'));
  return parts.join(' · ');
}

function kindChips(kinds) {
  const box = el('div', 'kchips');
  for (const k of KIND_ORDER.filter((x) => kinds.includes(x))) {
    const c = el('span', 'kchip');
    c.append(icon(KIND_ICON[k]), t('kind.' + k));
    box.append(c);
  }
  return box;
}

function presetCard(p) {
  const card = el('article', 'pcard' + (p.hidden ? ' off' : ''));
  const main = el('div', 'pc-main');
  main.append(el('h3', 'pc-name', p.name), el('p', 'pc-sum', presetSummary(p)), kindChips(p.kinds));
  const side = el('div', 'pc-side');
  const sw = switchEl(!p.hidden, t('pr.show_aria', { name: p.name }));
  sw.title = t(p.hidden ? 'pr.hidden' : 'pr.shown');
  sw.onchange = () => presetCall('set_preset_hidden', { id: p.id, hidden: !sw.checked });
  const vis = el('label', 'pc-vis');
  vis.append(el('span', '', t('pr.in_menu')), sw);
  const acts = el('div', 'pc-acts');
  if (!p.builtin) {
    const ed = iconBtn('i-edit', t('pr.edit_aria', { name: p.name }));
    ed.onclick = () => openPreset(p, false);
    acts.append(ed);
  }
  const dup = iconBtn('i-copy', t('pr.dup_aria', { name: p.name }));
  dup.onclick = () => openPreset(p, true);
  acts.append(dup);
  if (!p.builtin) {
    const del = iconBtn('i-trash', t('pr.del_aria', { name: p.name }), 'danger');
    del.onclick = async () => {
      if (await confirmModal({ title: 'pr.del_title', text: 'pr.del_text', yes: 'pr.del_yes' }, { name: p.name })) presetCall('delete_preset', { id: p.id });
    };
    acts.append(del);
  }
  side.append(vis, acts);
  card.append(main, side);
  card.dataset.id = p.id;
  return card;
}

function renderPresets() {
  const b = $('pr-builtin'), c = $('pr-custom');
  b.replaceChildren(...presets.filter((p) => p.builtin).map(presetCard));
  c.replaceChildren(...presets.filter((p) => !p.builtin).map(presetCard));
  $('pr-empty').hidden = presets.some((p) => !p.builtin);
}

async function presetCall(cmd, args) {
  try {
    presets = await invoke(cmd, args);
    banner('pr-err', '');
  } catch (e) {
    banner('pr-err', e);
  }
  renderPresets();
}

async function loadPresets() {
  try {
    presets = await invoke('list_presets');
    banner('pr-err', '');
  } catch (e) {
    banner('pr-err', t('err.presets', { err: e }));
  }
  renderPresets();
}

/* ---------- preset editor ---------- */
let pdId = '';
let pdKinds = new Set();

function buildPdSelects() {
  const tg = $('pd-target'), vt = $('pd-vtarget');
  const keepT = tg.value, keepV = vt.value;
  tg.replaceChildren(option('same', t('pd.keep')));
  vt.replaceChildren(option('', t('pd.vt_main')), option('same', t('pd.keep')));
  for (const k of KIND_ORDER) {
    const list = (catalog ? catalog.targets : []).filter((x) => x.kind === k);
    if (!list.length) continue;
    const g = el('optgroup');
    g.label = t('kind.' + k);
    list.forEach((x) => g.append(option(x.ext, x.ext.toUpperCase())));
    tg.append(g);
    if (k === 'video') list.forEach((x) => vt.append(option(x.ext, x.ext.toUpperCase())));
  }
  tg.value = keepT || 'jpg';
  vt.value = keepV;
  const kb = $('pd-kinds');
  kb.replaceChildren(...KIND_ORDER.map((k) => {
    const c = el('button', 'chip kchip-btn');
    c.type = 'button';
    c.dataset.kind = k;
    c.append(icon(KIND_ICON[k]), t('kind.' + k));
    c.onclick = () => {
      pdKinds.has(k) ? pdKinds.delete(k) : pdKinds.add(k);
      pdPaint();
    };
    return c;
  }));
}

function openPreset(p, dup) {
  buildPdSelects();
  pdId = p && !dup ? p.id : '';
  $('pd-title').textContent = t(!p ? 'pd.new_title' : dup ? 'pd.dup_title' : 'pd.edit_title');
  const o = (p && p.options) || {};
  $('pd-name').value = !p ? '' : dup ? t('pr.copy_name', { name: p.name }) : p.custom_name || p.name;
  pdKinds = new Set(p ? p.kinds : ['image']);
  $('pd-target').value = p ? p.target : 'jpg';
  $('pd-vtarget').value = (p && p.video_target) || '';
  const r = o.resize || {};
  const mode = r.max_edge != null ? 'max_edge' : r.percent != null ? 'percent' : r.exact ? 'exact' : 'none';
  $$('input[name=pd-resize]').forEach((x) => (x.checked = x.value === mode));
  $('pd-edge').value = r.max_edge ?? 1920;
  $('pd-pct').value = r.percent ?? 50;
  $('pd-w').value = r.exact ? r.exact.w : 1080;
  $('pd-h').value = r.exact ? r.exact.h : 1080;
  $('pd-fit').value = r.exact ? r.exact.fit : 'contain';
  $('pd-strip').checked = !!o.strip_metadata;
  $('pd-max-on').checked = !!o.max_bytes;
  const mb = !o.max_bytes || o.max_bytes >= 1e6;
  $('pd-unit').value = mb ? '1000000' : '1000';
  $('pd-max').value = o.max_bytes ? +(o.max_bytes / (mb ? 1e6 : 1e3)).toFixed(2) : 2;
  $('pd-q-on').checked = o.jpeg_quality != null;
  $('pd-q').value = o.jpeg_quality ?? (s ? s.quality.jpeg : 85);
  $('pd-v-on').checked = o.video_mbps != null;
  $('pd-v').value = o.video_mbps ?? (s ? s.quality.video_mbps : 8);
  ['pd-name-msg', 'pd-kinds-msg', 'pd-rs-msg', 'pd-opt-msg', 'pd-err'].forEach((x) => msg(x, ''));
  $('pd-name').removeAttribute('aria-invalid');
  pdPaint();
  openDlg('pd', $('pd-name'));
}

function pdCollect() {
  const mode = ($$('input[name=pd-resize]').find((x) => x.checked) || {}).value || 'none';
  const n = (id) => Math.round(+$(id).value);
  const resize = mode === 'max_edge' ? { max_edge: n('pd-edge') }
    : mode === 'percent' ? { percent: n('pd-pct') }
    : mode === 'exact' ? { exact: { w: n('pd-w'), h: n('pd-h'), fit: $('pd-fit').value } } : null;
  const kinds = KIND_ORDER.filter((k) => pdKinds.has(k));
  return {
    id: pdId,
    custom_name: $('pd-name').value.trim(),
    kinds,
    target: $('pd-target').value,
    video_target: kinds.includes('video') && $('pd-vtarget').value ? $('pd-vtarget').value : null,
    options: {
      resize,
      strip_metadata: $('pd-strip').checked,
      max_bytes: $('pd-max-on').checked ? Math.round(+$('pd-max').value * +$('pd-unit').value) : null,
      jpeg_quality: $('pd-q-on').checked ? +$('pd-q').value : null,
      video_mbps: $('pd-v-on').checked ? +$('pd-v').value : null,
    },
  };
}

// the same rules the backend applies (preset.rs validate), so most mistakes never reach it
function pdErrors(p) {
  const e = {};
  if (!p.custom_name || [...p.custom_name].length > 40) e.name = t('pd.e_name');
  if (!p.kinds.length) e.kinds = t('pd.e_kinds');
  const r = p.options.resize;
  const bad = (v, max) => !(v >= 1 && v <= max);
  if (r && ((r.max_edge != null && bad(r.max_edge, 16384)) || (r.percent != null && bad(r.percent, 400)) || (r.exact && (bad(r.exact.w, 16384) || bad(r.exact.h, 16384)))))
    e.resize = t(r.percent != null ? 'pd.e_pct' : 'pd.e_px');
  const o = p.options;
  if (o.max_bytes != null && !(o.max_bytes >= 1)) e.opt = t('pd.e_max');
  else if ((p.target === 'same' || p.video_target === 'same') && !r && !o.strip_metadata && o.max_bytes == null) e.opt = t('pd.e_same');
  return e;
}

function pdPaint(showErrors) {
  const mode = ($$('input[name=pd-resize]').find((x) => x.checked) || {}).value || 'none';
  paintSeg('pd-resize');
  $('pd-rs-edge').hidden = mode !== 'max_edge';
  $('pd-rs-pct').hidden = mode !== 'percent';
  $('pd-rs-exact').hidden = mode !== 'exact';
  $$('#pd-kinds .chip').forEach((c) => c.setAttribute('aria-pressed', pdKinds.has(c.dataset.kind)));
  $('pd-vt-wrap').hidden = !pdKinds.has('video');
  $('pd-max-wrap').hidden = !$('pd-max-on').checked;
  $('pd-q').hidden = !$('pd-q-on').checked;
  $('pd-v').hidden = !$('pd-v-on').checked;
  paintRange($('pd-q'), $('pd-q-val'));
  paintRange($('pd-v'), $('pd-v-val'), 'Mb/s');
  const p = pdCollect();
  $('pd-sum').textContent = presetSummary(p);
  if (!showErrors) return p;
  const e = pdErrors(p);
  msg('pd-name-msg', e.name);
  $('pd-name').setAttribute('aria-invalid', !!e.name);
  msg('pd-kinds-msg', e.kinds);
  msg('pd-rs-msg', e.resize);
  msg('pd-opt-msg', e.opt);
  return Object.keys(e).length ? null : p;
}

let pdTried = false;
async function pdSave(ev) {
  ev.preventDefault();
  pdTried = true;
  const p = pdPaint(true);
  msg('pd-err', '');
  if (!p) {
    const bad = document.querySelector('#pd [aria-invalid=true]');
    return bad ? bad.focus() : showFirstError('pd');
  }
  const b = $('pd-save');
  b.disabled = true;
  try {
    presets = await invoke('save_preset', { preset: p });
    renderPresets();
    closeDlg();
  } catch (e) {
    // backend errors are plain localized text; shown next to Save
    msg('pd-err', String(e));
  }
  b.disabled = false;
}

/* ======================================================================== history */
function relTime(ms) {
  const sec = (Date.now() - ms) / 1000;
  if (sec < 45) return t('hi.just_now');
  const rtf = new Intl.RelativeTimeFormat(LANG, { numeric: 'auto' });
  if (sec < 3600) return rtf.format(-Math.round(sec / 60), 'minute');
  if (sec < 86400) return rtf.format(-Math.round(sec / 3600), 'hour');
  if (sec < 7 * 86400) return rtf.format(-Math.round(sec / 86400), 'day');
  return new Date(ms).toLocaleDateString(LANG, { day: 'numeric', month: 'short', year: 'numeric' });
}

// cancelled jobs are stored as failures with the backend's "Cancelled" text
const histCancelled = (e) => !e.ok && Object.values(I18N).some((d) => d['p.cancelled'] === e.error);

function histRow(e) {
  const cancelled = histCancelled(e);
  const li = el('li', 'hrow' + (e.ok || cancelled ? '' : ' is-err') + (e.undone ? ' is-undone' : ''));
  const ext = extOf(e.input), out = extOf(e.output);
  const tile = el('span', 'tile sm');
  tile.append(icon(KIND_ICON[kindOfExt(ext)]));
  const main = el('div', 'grow');
  const name = el('div', 'h-name', fileName(e.input));
  name.title = e.input;
  const meta = el('div', 'h-meta');
  if (ext) meta.append(el('span', 'fmt', ext.toUpperCase()));
  if (out) meta.append(icon('i-arrow', 'ic tiny'), el('span', 'fmt to', out.toUpperCase()));
  const time = el('time', 'h-time', relTime(e.time));
  time.dateTime = new Date(e.time).toISOString();
  time.title = new Date(e.time).toLocaleString(LANG);
  meta.append(time);
  if (e.kind !== 'convert' && e.label) {
    const k = el('span', 'kbadge');
    k.append(icon({ preset: 'i-wand', merge: 'i-merge', watch: 'i-eye' }[e.kind] || 'i-files'), e.label);
    k.title = t({ preset: 'hi.kind_preset', merge: 'hi.kind_merge', watch: 'hi.kind_watch' }[e.kind] || 'hi.kind_preset');
    meta.append(k);
  }
  main.append(name, meta);
  if (!e.ok && !cancelled) main.append(el('p', 'h-msg err', e.error || t('p.fail_default')));
  const um = undoMsg.get(e.id);
  if (um) main.append(el('p', 'h-msg ' + (um[0] ? 'ok' : 'err'), um[1]));
  const acts = el('div', 'h-acts');
  if (e.undone) acts.append(el('span', 'pill', t('hi.undone')));
  else if (cancelled) acts.append(el('span', 'pill', t('hi.cancelled')));
  else if (!e.ok) acts.append(el('span', 'pill err', t('hi.failed')));
  else {
    const u = textBtn('i-undo', t('hi.undo'), 'ghost undo');
    u.setAttribute('aria-label', t('hi.undo_aria', { name: fileName(e.input) }));
    u.disabled = !e.output;
    if (!e.output) u.title = t('hi.no_output');
    u.onclick = () => undo(e, u);
    acts.append(u);
  }
  const open = iconBtn('i-folder', t('p.open'));
  open.onclick = () => invoke('open_folder', { path: e.ok && !e.undone && e.output ? e.output : dirOf(e.input) })
    .catch((err) => (undoMsg.set(e.id, [false, t('p.open_err', { err })]), renderHistory()));
  acts.append(open);
  li.append(tile, main, acts);
  li.dataset.id = e.id;
  return li;
}

async function undo(e, btn) {
  btn.disabled = true;
  btn.classList.add('busy');
  try {
    await invoke('undo_job', { id: e.id });
    e.undone = true;
    undoMsg.set(e.id, [true, t('hi.undo_ok')]);
  } catch (err) {
    undoMsg.set(e.id, [false, String(err)]);
  }
  renderHistory();
  // the row was rebuilt: keep keyboard focus on it
  const row = $('hist').querySelector(`[data-id="${e.id}"]`);
  if (row) (row.querySelector('.undo') || row.querySelector('.iconbtn')).focus();
}

function renderHistory() {
  const list = $('hist');
  const f = document.createDocumentFragment();
  hist.forEach((e) => f.append(histRow(e)));
  list.replaceChildren(f);
  const empty = !hist.length;
  list.hidden = empty;
  $('hist-empty').hidden = !empty;
  $('hist-clear').hidden = empty;
  $('hist-count').textContent = empty ? '' : t('hi.count', { n: hist.length });
  const off = s && !s.history_enabled;
  $('hist-empty-title').textContent = t(off ? 'hi.off_title' : 'hi.empty_title');
  $('hist-empty-text').textContent = t(off ? 'hi.off_text' : 'hi.empty_text');
}

async function loadHistory() {
  try {
    hist = await invoke('get_history');
    banner('hist-err', '');
  } catch (e) {
    banner('hist-err', t('err.history', { err: e }));
  }
  renderHistory();
}

/* ======================================================================== watch */
function outText(o) {
  if (o.startsWith('subfolder:')) return t('wa.o_sub', { name: o.slice(10) });
  if (o.startsWith('fixed:')) return t('wa.o_fixed', { path: o.slice(6) });
  return t('wa.o_same');
}

function actText(a) {
  if (a.kind === 'preset') {
    const p = presets.find((x) => x.id === a.preset_id);
    return t('wa.a_preset', { name: p ? p.name : a.preset_id });
  }
  return t('wa.a_target', { fmt: a.target.toUpperCase() });
}

function ruleCard(r, i) {
  const st = watch.status.find((x) => x.id === r.id);
  const err = st && !st.ok && st.error;
  const li = el('li', 'rcard' + (r.enabled ? '' : ' off') + (err ? ' is-err' : ''));
  const tile = el('span', 'tile' + (err ? ' danger' : ''));
  tile.append(icon(err ? 'i-alert' : 'i-folder'));
  const main = el('div', 'grow');
  const name = el('div', 'r-name', fileName(r.folder) || r.folder);
  const path = el('div', 'r-path', r.folder);
  path.title = r.folder;
  const tags = el('div', 'r-tags');
  const tag = (ic, text) => {
    const x = el('span', 'kchip');
    x.append(icon(ic), text);
    tags.append(x);
  };
  tag('i-wand', actText(r.action));
  const exts = r.exts.map((e) => e.toUpperCase());
  tag('i-files', !exts.length ? t('wa.all_types') : exts.length > 4 ? t('wa.types_more', { list: exts.slice(0, 3).join(', '), n: exts.length - 3 }) : exts.join(', '));
  if (r.recursive) tag('i-folder', t('wa.recursive'));
  tag('i-arrow', outText(r.output));
  main.append(name, path, tags);
  if (err) main.append(el('p', 'h-msg err', err));
  else if (watch.enabled && r.enabled && st) {
    const ok = el('p', 'r-live');
    ok.append(icon('i-eye', 'ic tiny'), t('wa.live'));
    main.append(ok);
  }
  const side = el('div', 'pc-side');
  const sw = switchEl(r.enabled, t('wa.enable_aria', { name: fileName(r.folder) || r.folder }));
  sw.onchange = () => saveWatch({ ...watch, rules: watch.rules.map((x, k) => (k === i ? { ...x, enabled: sw.checked } : x)) });
  const acts = el('div', 'pc-acts');
  const ed = iconBtn('i-edit', t('wa.edit_aria', { name: fileName(r.folder) }));
  ed.onclick = () => openRule(i);
  const del = iconBtn('i-trash', t('wa.del_aria', { name: fileName(r.folder) }), 'danger');
  del.onclick = async () => {
    if (await confirmModal({ title: 'wa.del_title', text: 'wa.del_text', yes: 'wa.del_yes' }, { name: r.folder }))
      saveWatch({ ...watch, rules: watch.rules.filter((_, k) => k !== i) });
  };
  acts.append(ed, del);
  side.append(sw, acts);
  li.append(tile, main, side);
  return li;
}

function renderWatch() {
  $('watch-sw').checked = watch.enabled;
  $('rules').replaceChildren(...watch.rules.map(ruleCard));
  const n = watch.rules.length;
  $('rules').hidden = !n;
  $('wa-empty').hidden = !!n;
  $('wa-off').hidden = watch.enabled || !n;
  $('wa-count').textContent = t('wa.count', { n, max: MAX_RULES });
  $('wa-add').disabled = n >= MAX_RULES;
  $('wa-add').title = n >= MAX_RULES ? t('wa.full', { max: MAX_RULES }) : '';
}

async function loadWatch() {
  try {
    watch = await invoke('get_watch');
    banner('wa-err', '');
  } catch (e) {
    banner('wa-err', t('err.watch', { err: e }));
  }
  renderWatch();
}

// saves the whole config; the backend answers with the normalized copy, status comes from get_watch
async function saveWatch(next, fromDialog) {
  try {
    const w = await invoke('set_watch', { watch: { enabled: next.enabled, rules: next.rules } });
    watch = { ...w, status: watch.status };
    banner('wa-err', '');
    try {
      watch.status = (await invoke('get_watch')).status;
    } catch { /* keep the old status */ }
    renderWatch();
    return true;
  } catch (e) {
    if (fromDialog) msg('wd-err', String(e));
    else banner('wa-err', e);
    renderWatch(); // switches snap back to the saved state
    return false;
  }
}

/* ---------- rule editor ---------- */
let wdIndex = -1;
let wd = null; // draft rule

function openRule(i) {
  wdIndex = i;
  const r = i >= 0 ? watch.rules[i] : null;
  const firstPreset = (presets.find((p) => !p.hidden) || presets[0] || { id: 'web' }).id;
  wd = r ? structuredClone(r) : { id: '', enabled: true, folder: '', recursive: false, exts: [], action: { kind: 'preset', preset_id: firstPreset }, output: 'same_folder' };
  $('wd-title').textContent = t(r ? 'wd.edit_title' : 'wd.new_title');
  $('wd-preset').replaceChildren(...presets.map((p) => option(p.id, p.name)));
  const tg = $('wd-target');
  tg.replaceChildren();
  for (const k of KIND_ORDER) {
    const list = (catalog ? catalog.targets : []).filter((x) => x.kind === k);
    if (!list.length) continue;
    const g = el('optgroup');
    g.label = t('kind.' + k);
    list.forEach((x) => g.append(option(x.ext, x.ext.toUpperCase())));
    tg.append(g);
  }
  $('wd-preset').value = wd.action.kind === 'preset' ? wd.action.preset_id : firstPreset;
  tg.value = wd.action.kind === 'target' ? wd.action.target : 'jpg';
  $$('input[name=wd-act]').forEach((x) => (x.checked = x.value === wd.action.kind));
  $('wd-rec').checked = wd.recursive;
  $('wd-all').checked = !wd.exts.length;
  const seen = new Set();
  $('wd-exts').replaceChildren(...(catalog ? catalog.sources : []).filter((x) => !seen.has(x.ext) && seen.add(x.ext)).map((x) => {
    const c = el('button', 'chip', x.ext.toUpperCase());
    c.type = 'button';
    c.dataset.ext = x.ext;
    c.onclick = () => {
      wd.exts = wd.exts.includes(x.ext) ? wd.exts.filter((e) => e !== x.ext) : [...wd.exts, x.ext];
      wdPaint();
    };
    return c;
  }));
  const o = wd.output;
  const kind = o.startsWith('subfolder:') ? 'sub' : o.startsWith('fixed:') ? 'fixed' : 'same';
  $$('input[name=wd-out]').forEach((x) => (x.checked = x.value === kind));
  $('wd-sub').value = kind === 'sub' ? o.slice(10) : t('wd.sub_default');
  wd.fixed = kind === 'fixed' ? o.slice(6) : '';
  ['wd-folder-msg', 'wd-exts-msg', 'wd-out-msg', 'wd-err'].forEach((x) => msg(x, ''));
  wdPaint();
  openDlg('wd', $('wd-pick'));
}

function wdPaint() {
  const f = $('wd-folder');
  f.textContent = wd.folder || t('wd.no_folder');
  f.classList.toggle('empty-path', !wd.folder);
  const all = $('wd-all').checked;
  $('wd-exts').hidden = all;
  $$('#wd-exts .chip').forEach((c) => c.setAttribute('aria-pressed', wd.exts.includes(c.dataset.ext)));
  const act = ($$('input[name=wd-act]').find((x) => x.checked) || {}).value;
  paintSeg('wd-act');
  $('wd-preset').parentElement.hidden = act !== 'preset';
  $('wd-target').parentElement.hidden = act !== 'target';
  const out = ($$('input[name=wd-out]').find((x) => x.checked) || {}).value;
  $('wd-sub').disabled = out !== 'sub';
  $('wd-opath').hidden = out !== 'fixed' || !wd.fixed;
  $('wd-opath').textContent = wd.fixed;
}

function wdCollect() {
  const act = ($$('input[name=wd-act]').find((x) => x.checked) || {}).value;
  const out = ($$('input[name=wd-out]').find((x) => x.checked) || {}).value;
  const errs = {};
  if (!wd.folder) errs.folder = t('wd.e_folder');
  if (!$('wd-all').checked && !wd.exts.length) errs.exts = t('wd.e_exts');
  const sub = $('wd-sub').value.trim();
  if (out === 'sub' && (!sub || /[\\/:*?"<>|{}]/.test(sub) || /[. ]$/.test(sub))) errs.out = t('wd.e_sub');
  if (out === 'fixed' && !wd.fixed) errs.out = t('wd.e_fixed');
  msg('wd-folder-msg', errs.folder);
  msg('wd-exts-msg', errs.exts);
  msg('wd-out-msg', errs.out);
  if (Object.keys(errs).length) return null;
  return {
    id: wd.id,
    enabled: wd.enabled,
    folder: wd.folder,
    recursive: $('wd-rec').checked,
    exts: $('wd-all').checked ? [] : catalog.sources.map((x) => x.ext).filter((e, k, a) => wd.exts.includes(e) && a.indexOf(e) === k),
    action: act === 'preset' ? { kind: 'preset', preset_id: $('wd-preset').value } : { kind: 'target', target: $('wd-target').value },
    output: out === 'sub' ? 'subfolder:' + sub : out === 'fixed' ? 'fixed:' + wd.fixed : 'same_folder',
  };
}

async function wdSave(ev) {
  ev.preventDefault();
  msg('wd-err', '');
  const r = wdCollect();
  if (!r) return showFirstError('wd');
  const rules = wdIndex >= 0 ? watch.rules.map((x, k) => (k === wdIndex ? r : x)) : [...watch.rules, r];
  const b = $('wd-save');
  b.disabled = true;
  if (await saveWatch({ ...watch, rules }, true)) closeDlg();
  b.disabled = false;
}

/* ======================================================================== updates */
function paintUpdate() {
  const u = lastUpdate, res = $('upd-result'), tile = $('upd-tile');
  $('upd-open').hidden = !(u && u.available);
  tile.className = 'tile upd-tile';
  if (!u) return (res.textContent = t('upd.never')), $('upd-ic').setAttribute('href', '#i-retry');
  const none = !u.error && !u.available && /\/releases\/?$/.test(u.url || '');
  const state = u.error ? 'err' : u.available ? 'new' : 'ok';
  tile.classList.add(state);
  $('upd-ic').setAttribute('href', { err: '#i-alert', new: '#i-download', ok: '#i-check' }[state]);
  res.textContent = u.error ? u.error
    : u.available ? t('upd.avail', { latest: u.latest, current: u.current })
    : none ? t('upd.none', { current: u.current }) : t('upd.ok', { current: u.current });
}

async function checkUpdate() {
  const b = $('upd-check');
  b.disabled = true;
  b.classList.add('busy');
  b.textContent = t('upd.checking');
  try {
    lastUpdate = await invoke('check_update');
  } catch (e) {
    lastUpdate = { error: String(e) };
  }
  b.disabled = false;
  b.classList.remove('busy');
  b.textContent = t('upd.check');
  paintUpdate();
}

async function openRelease() {
  try {
    await invoke('open_url', { url: lastUpdate.url });
  } catch (e) {
    lastUpdate = { ...lastUpdate, available: false, error: String(e) };
    paintUpdate();
  }
}

function updateBanner(info) {
  lastUpdate = info;
  paintUpdate();
  $('upd-banner-text').textContent = t('upd.banner', { latest: info.latest });
  $('upd-banner').hidden = false;
}

/* ======================================================================== command line */
function renderCli() {
  const exe = document.title;
  const rows = [
    [`${exe} convert photo.heic --to jpg`, 'cli.c_heic'],
    [`${exe} convert scan.png --to webp --max-edge 1600 --strip`, 'cli.c_web'],
    [`${exe} convert clip.mov --to mp4 --max-size 8MB`, 'cli.c_mail'],
    [`${exe} preset whatsapp beach.jpg`, 'cli.c_preset'],
    [`${exe} list-presets`, 'cli.c_list'],
    [`${exe} formats heic`, 'cli.c_formats'],
  ];
  $('cli').replaceChildren(...rows.map(([cmd, key]) => {
    const li = el('li', 'cli-row');
    const txt = el('div', 'grow');
    const code = el('code', 'cmd');
    // words never break inside ("--max-edge" stays whole), lines wrap between them
    cmd.split(' ').forEach((w, i) => code.append(i ? ' ' : '', el('span', '', w)));
    txt.append(code, el('p', 'desc', t(key)));
    const b = iconBtn('i-copy', t('cli.copy'));
    b.onclick = () => copy(cmd, b);
    li.append(txt, b);
    return li;
  }));
}

async function copy(text, b) {
  let ok = false;
  try {
    await navigator.clipboard.writeText(text);
    ok = true;
  } catch {
    // clipboard API refused: select the text and use the old command
    const r = document.createRange();
    r.selectNodeContents(b.previousElementSibling.firstElementChild);
    getSelection().removeAllRanges();
    getSelection().addRange(r);
    ok = document.execCommand('copy');
  }
  const label = t(ok ? 'cli.copied' : 'cli.copy_fail');
  b.firstElementChild.firstElementChild.setAttribute('href', ok ? '#i-check' : '#i-alert');
  b.classList.toggle('done', ok);
  b.title = label;
  b.setAttribute('aria-label', label);
  clearTimeout(b.timer);
  b.timer = setTimeout(() => {
    b.firstElementChild.firstElementChild.setAttribute('href', '#i-copy');
    b.classList.remove('done');
    b.title = t('cli.copy');
    b.setAttribute('aria-label', t('cli.copy'));
  }, 1600);
}

/* ======================================================================== wiring */
let clock = null;

// app.js calls this when a section opens
function onSection(id) {
  clearInterval(clock);
  if (id === 't-history') {
    renderHistory(); // fresh relative times
    clock = setInterval(renderHistory, 60000);
  }
}

function featuresRelabel() {
  renderCli();
  paintUpdate();
  if (!$('upd-banner').hidden && lastUpdate) $('upd-banner-text').textContent = t('upd.banner', { latest: lastUpdate.latest });
  renderHistory();
  renderPresets();
  renderWatch();
}

function featuresInit() {
  $('pr-new').onclick = () => openPreset(null, false);
  $('pd-form').onsubmit = pdSave;
  $('pd-form').addEventListener('input', () => pdPaint(pdTried));
  $('pd-form').addEventListener('change', () => pdPaint(pdTried));
  $('wd-form').onsubmit = wdSave;
  $('wd-form').addEventListener('change', wdPaint);
  $$('[data-close]').forEach((b) => (b.onclick = closeDlg));
  $('wd-pick').onclick = async () => {
    const p = await pickDir();
    if (p) (wd.folder = p), msg('wd-folder-msg', ''), wdPaint();
  };
  $('wd-opick').onclick = async () => {
    const p = await pickDir();
    if (p) {
      wd.fixed = p;
      $$('input[name=wd-out]').forEach((x) => (x.checked = x.value === 'fixed'));
      msg('wd-out-msg', '');
      wdPaint();
    }
  };
  $('watch-sw').onchange = (e) => saveWatch({ ...watch, enabled: e.target.checked });
  $('wa-add').onclick = () => openRule(-1);
  $('hist-clear').onclick = async () => {
    if (!(await confirmModal({ title: 'hi.clear_title', text: 'hi.clear_text', yes: 'hi.clear_yes' }))) return;
    try {
      await invoke('clear_history');
      undoMsg.clear();
    } catch (e) {
      banner('hist-err', e);
    }
    loadHistory();
  };
  $('hist-sw').addEventListener('change', () => setTimeout(renderHistory, 0));
  $('upd-check').onclick = checkUpdate;
  $('upd-open').onclick = openRelease;
  $('upd-banner-get').onclick = openRelease;
  $('upd-banner-x').onclick = () => ($('upd-banner').hidden = true);
  if (T && T.event) {
    T.event.listen('history-changed', loadHistory);
    T.event.listen('update-available', (e) => updateBanner(e.payload));
    // built-in preset names and watch errors are written by the backend in its language.
    // ponytail: it restarts the watchers right after announcing the language; the short wait picks up their new status
    T.event.listen('language-changed', () => {
      loadPresets().then(renderWatch);
      setTimeout(loadWatch, 400);
    });
  }
  renderCli();
  paintUpdate();
  loadPresets().then(() => loadWatch());
  loadHistory();
}
