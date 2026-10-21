'use strict';
const T = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const SVG = 'http://www.w3.org/2000/svg';
const invoke = (cmd, args) =>
  T && T.core ? T.core.invoke(cmd, args) : Promise.reject(t('err.backend'));

let s = null; // current settings object
let catalog = null; // {sources, targets}
let ffReady = false;
let saveTimer, savedTimer, toastTimer, ffTimer, hintTimer, lastField, goodTpl;

$('app-title').textContent = document.title;
setLang('en'); // until settings say otherwise

// ponytail: fills only what an older settings.json may lack; the backend owns the real defaults
function normalize(x) {
  x.language = x.language === 'tr' ? 'tr' : 'en';
  x.quality = { jpeg: 85, mp3_q: 2, ogg_q: 6, video_mbps: 8, gif_fps: 15, gif_width: 480, ...x.quality };
  x.panel = { corner: 'top_right', seconds: 7, windows_toast: false, ...x.panel };
  x.menu = { icons: true, sort: 'default', presets_enabled: true, merge: true, ...x.menu };
  x.updates = { check_on_start: false, ...x.updates };
  x.history_enabled ??= true;
  x.enabled_targets ||= [];
  x.menu_sources ||= [];
  x.name_template ??= '{name}';
  return x;
}

function showError(msg) {
  $('error').hidden = !msg;
  $('error').textContent = msg ? String(msg) : '';
}

function icon(id, cls) {
  const i = document.createElementNS(SVG, 'svg');
  i.setAttribute('class', cls || 'ic');
  i.setAttribute('aria-hidden', 'true');
  const u = document.createElementNS(SVG, 'use');
  u.setAttribute('href', '#' + id);
  i.append(u);
  return i;
}

/* ---------- sidebar sections ---------- */
const tabs = $$('[role=tab]');
let cur = 0;

// the paper thumb slides to the chosen row; the header names the section
function paintNav() {
  const tb = tabs[cur], nav = $('tabs');
  nav.style.setProperty('--y', tb.offsetTop + 'px');
  nav.style.setProperty('--h', tb.offsetHeight + 'px');
  $('pane-title').textContent = tb.textContent;
  $('pane-desc').textContent = t(tb.dataset.desc);
}

function select(i, focus) {
  if (focus) tabs[i].focus();
  if (i === cur) return;
  const dir = i > cur ? 1 : -1;
  tabs.forEach((t, k) => {
    const on = k === i;
    t.setAttribute('aria-selected', on);
    t.tabIndex = on ? 0 : -1;
    const p = $(t.getAttribute('aria-controls'));
    p.hidden = !on;
    p.classList.toggle('on', on);
  });
  const p = $(tabs[i].getAttribute('aria-controls'));
  p.style.setProperty('--dir', dir);
  p.classList.remove('in');
  void p.offsetWidth; // restart the slide
  p.classList.add('in');
  cur = i;
  paintNav();
  $('main').scrollTo(0, 0);
  if (typeof onSection === 'function') onSection(tabs[i].id);
}

function tabKeys(e) {
  const k = { ArrowDown: cur + 1, ArrowRight: cur + 1, ArrowUp: cur - 1, ArrowLeft: cur - 1, Home: 0, End: tabs.length - 1 }[e.key];
  if (k == null) return;
  e.preventDefault();
  select((k + tabs.length) % tabs.length, true);
}

/* ---------- save ---------- */
function save(field) {
  lastField = field;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    const f = lastField;
    try {
      await invoke('set_settings', { settings: s });
      goodTpl = s.name_template;
      showError('');
      if (f === 'tpl') tplMsg('');
      $('saved').classList.add('on');
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => $('saved').classList.remove('on'), 1600);
    } catch (e) {
      // a rejected template must not ride along with the next unrelated save
      if (f === 'tpl') tplMsg(String(e)), (s.name_template = goodTpl);
      else showError(t('err.save', { err: e }));
    }
  }, 300);
}

function change(patch, field) {
  Object.assign(s, patch);
  render();
  save(field);
}

function setIn(group, key, val, field) {
  s[group] = { ...s[group], [key]: val };
  render();
  save(field);
}

/* ---------- sliders ---------- */
const MP3_KBPS = [245, 225, 190, 175, 165, 130, 115, 100, 85, 65]; // lame -V0..-V9 averages
const OGG_KBPS = [64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 500]; // vorbis -q0..-q10 nominal
const SLIDERS = {
  'q-jpeg': { get: () => s.quality.jpeg, set: (v) => setIn('quality', 'jpeg', v), show: (v) => [v, ''] },
  // mp3_q: 0 is best, so the slider runs inverted and stores the raw value
  'q-mp3': { get: () => 9 - s.quality.mp3_q, set: (v) => setIn('quality', 'mp3_q', 9 - v), show: (v) => ['≈' + MP3_KBPS[9 - v], 'kb/s'] },
  'q-ogg': { get: () => s.quality.ogg_q, set: (v) => setIn('quality', 'ogg_q', v), show: (v) => ['≈' + OGG_KBPS[v], 'kb/s'] },
  'q-video': { get: () => s.quality.video_mbps, set: (v) => setIn('quality', 'video_mbps', v), show: (v) => [v, 'Mb/s'] },
  'q-fps': { get: () => s.quality.gif_fps, set: (v) => setIn('quality', 'gif_fps', v), show: (v) => [v, 'fps'] },
  'q-width': { get: () => s.quality.gif_width, set: (v) => setIn('quality', 'gif_width', v), show: (v) => [v, 'px'] },
  'q-secs': { get: () => s.panel.seconds, set: (v) => setIn('panel', 'seconds', v), show: (v) => [v, t('unit.sec')] },
};

function paintSlider(id, v) {
  const el = $(id);
  const [n, unit] = SLIDERS[id].show(v);
  el.value = v;
  el.style.setProperty('--fill', ((v - el.min) / (el.max - el.min)) * 100 + '%');
  el.setAttribute('aria-valuetext', unit ? `${n} ${unit}` : String(n));
  const out = el.closest('.q').querySelector('output');
  out.firstElementChild.textContent = n;
  out.lastElementChild.textContent = unit;
}

/* ---------- formats ---------- */
const KINDS = [['image', 'kind.image', 'i-image'], ['audio', 'kind.audio', 'i-audio'], ['video', 'kind.video', 'i-film'], ['document', 'kind.document', 'i-doc']];
// ponytail: the catalog has no needs_ffmpeg flag for sources; these decode through ffmpeg (video too). Drop when get_catalog grows one.
const FF_SOURCES = ['heic', 'heif', 'avif'];
const RAW = ['dng', 'cr2', 'cr3', 'nef', 'arw', 'orf', 'rw2', 'raf'];
const srcNeedsFf = (x) => x.kind === 'video' || FF_SOURCES.includes(x.ext);

function buildKinds(box, items, sources) {
  box.replaceChildren();
  for (const [kind, titleKey, ic] of KINDS) {
    const title = t(titleKey);
    const list = items.filter((x) => x.kind === kind);
    if (!list.length) continue;
    const row = document.createElement('div');
    row.className = 'row top-align kind';
    const tile = document.createElement('span');
    tile.className = 'tile';
    tile.append(icon(ic));
    const grow = document.createElement('div');
    grow.className = 'grow';
    const line = document.createElement('div');
    line.className = 'label-line';
    const lab = document.createElement('span');
    lab.className = 'label';
    lab.id = box.id + '-' + kind;
    lab.textContent = title;
    line.append(lab);
    if (sources) {
      const acts = document.createElement('span');
      acts.className = 'mini';
      for (const [txt, all] of [[t('all.btn'), true], [t('none.btn'), false]]) {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'link';
        b.textContent = txt;
        b.setAttribute('aria-label', t(all ? 'all.aria' : 'none.aria', { kind: title }));
        b.onclick = () => groupSources(kind, all);
        acts.append(b);
      }
      line.append(acts);
    }
    const chips = document.createElement('div');
    chips.className = 'chips';
    chips.setAttribute('role', 'group');
    chips.setAttribute('aria-labelledby', lab.id);
    for (const it of list) {
      const c = document.createElement('button');
      c.type = 'button';
      c.className = 'chip';
      c.dataset.ext = it.ext;
      c.append(it.ext.toUpperCase());
      if (sources ? srcNeedsFf(it) : it.needs_ffmpeg) {
        c.dataset.ff = '';
        c.append(icon('i-download', 'ic badge'));
      }
      if (sources) c.title = srcNeedsFf(it) ? t('chip.src_ff') : RAW.includes(it.ext) ? t('chip.raw') : '';
      c.onclick = () => (sources ? toggleSource(it.ext) : toggleTarget(it, c));
      chips.append(c);
    }
    grow.append(line, chips);
    row.append(tile, grow);
    box.append(row);
  }
}

function paintChips() {
  if (!catalog || !s) return;
  for (const c of $$('#targets .chip')) {
    const on = s.enabled_targets.includes(c.dataset.ext);
    const locked = 'ff' in c.dataset && !ffReady;
    c.setAttribute('aria-pressed', on);
    c.setAttribute('aria-disabled', locked);
    c.title = locked ? t('chip.locked') : 'ff' in c.dataset ? t('chip.ff') : '';
  }
  for (const c of $$('#sources .chip')) c.setAttribute('aria-pressed', s.menu_sources.includes(c.dataset.ext));
  paintHints();
}

function hint(el, kind, parts) {
  el.className = 'foot hint' + (kind ? ' ' + kind : '');
  el.replaceChildren(...parts);
}

function paintHints(flash) {
  const th = $('t-hint');
  const anyFf = catalog.targets.some((t) => t.needs_ffmpeg);
  if (flash) {
    hint(th, 'warn', [icon('i-alert'), flash]);
    clearTimeout(hintTimer);
    hintTimer = setTimeout(() => paintHints(), 3200);
  } else if (!s.enabled_targets.length) {
    hint(th, 'warn', [icon('i-alert'), t('hint.none_targets')]);
  } else if (anyFf && !ffReady) {
    hint(th, '', [icon('i-download', 'ic badge'), t('hint.ff_wait'), gotoLink(t('hint.see_status'), 't-ileri')]);
  } else if (anyFf) {
    hint(th, '', [icon('i-download', 'ic badge'), t('hint.ff_ready')]);
  } else hint(th, '', []);
  th.hidden = !th.childNodes.length;
  const n = s.menu_sources.length;
  const raw = s.menu_sources.some((e) => RAW.includes(e));
  hint($('s-hint'), n ? '' : 'warn', n
    ? [t('hint.sources', { n }), ' ', icon('i-download', 'ic badge'), t('hint.src_ff'), ...(raw ? [' ' + t('hint.raw')] : [])]
    : [icon('i-alert'), t('hint.none_sources')]);
}

function gotoLink(text, tab) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'link';
  b.dataset.goto = tab;
  b.textContent = text;
  return b;
}

function toggleTarget(it, chip) {
  if (!ffReady && it.needs_ffmpeg) return paintHints(t('flash.ff_needed', { fmt: it.ext.toUpperCase() }));
  const on = s.enabled_targets.includes(it.ext);
  if (on && s.enabled_targets.length === 1) {
    chip.classList.remove('nudge');
    void chip.offsetWidth;
    chip.classList.add('nudge');
    return paintHints(t('flash.last'));
  }
  const set = new Set(s.enabled_targets);
  on ? set.delete(it.ext) : set.add(it.ext);
  change({ enabled_targets: catalog.targets.map((t) => t.ext).filter((e) => set.has(e)) });
}

function toggleSource(ext) {
  const set = new Set(s.menu_sources);
  set.has(ext) ? set.delete(ext) : set.add(ext);
  change({ menu_sources: catalog.sources.map((x) => x.ext).filter((e) => set.has(e)) });
}

function groupSources(kind, all) {
  const set = new Set(s.menu_sources);
  catalog.sources.filter((x) => x.kind === kind).forEach((x) => (all ? set.add(x.ext) : set.delete(x.ext)));
  change({ menu_sources: catalog.sources.map((x) => x.ext).filter((e) => set.has(e)) });
}

async function loadCatalog() {
  try {
    catalog = await invoke('get_catalog');
    buildKinds($('targets'), catalog.targets, false);
    buildKinds($('sources'), catalog.sources, true);
    paintChips();
    tplPaint();
  } catch (e) {
    $('cat-err').hidden = false;
    $('cat-err').textContent = t('err.catalog', { err: e });
  }
}

/* ---------- name template ---------- */
const TOKENS = ['{name}', '{format}', '{date}'];

function tplError(x) {
  if (!x.trim()) return t('tpl.empty');
  if (/[\\/:*?"<>|]/.test(x)) return t('tpl.chars');
  const bad = (x.match(/\{[^{}]*\}/g) || []).find((k) => !TOKENS.includes(k));
  if (bad) return t('tpl.unknown', { tag: bad });
  if (/[{}]/.test(x.replace(/\{[^{}]*\}/g, ''))) return t('tpl.braces');
  if (/[. ]$/.test(x)) return t('tpl.trailing');
  return ''; // reserved names (CON, NUL...) are left to the backend's message
}

function tplMsg(msg) {
  $('tpl-msg').textContent = msg;
  $('tpl').setAttribute('aria-invalid', !!msg);
  $('tpl-ex').classList.toggle('stale', !!msg);
}

function tplPaint() {
  const v = $('tpl').value;
  const err = tplError(v);
  tplMsg(err);
  if (err) return;
  const img = catalog ? catalog.targets.find((x) => x.kind === 'image' && x.ext !== 'png' && s.enabled_targets.includes(x.ext)) : null;
  const ext = img ? img.ext : 'jpg';
  const d = new Date();
  const date = [d.getFullYear(), d.getMonth() + 1, d.getDate()].map((x) => String(x).padStart(2, '0')).join('-');
  $('tpl-out').textContent = v.replaceAll('{name}', t('tpl.sample')).replaceAll('{format}', ext).replaceAll('{date}', date) + '.' + ext;
}

function tplInput() {
  tplPaint();
  const v = $('tpl').value;
  if (!tplError(v) && v !== s.name_template) {
    s.name_template = v;
    save('tpl');
  }
}

/* ---------- panel preview ---------- */
let demoTimer = null;

// the real mini panel plays a fake job on the desktop; the button rests while it runs
async function demoPlay() {
  const b = $('demo-play');
  if (b.disabled) return;
  b.disabled = true;
  clearTimeout(demoTimer);
  demoTimer = setTimeout(() => (b.disabled = false), 3600);
  try {
    await invoke('panel_preview', { corner: s.panel.corner, seconds: s.panel.seconds });
  } catch (e) {
    showError(t('err.preview', { err: e }));
  }
}

/* ---------- render ---------- */
function seg(el, i) {
  el.style.setProperty('--i', i);
}

function render() {
  $('autostart').checked = s.autostart;
  const fixed = s.output_dir.kind === 'fixed';
  document.querySelector(`input[name=dir][value=${fixed ? 'fixed' : 'same_as_source'}]`).checked = true;
  $('path').hidden = !fixed;
  $('path').textContent = fixed ? s.output_dir.path || '' : '';
  $('conflict').value = s.conflict;
  $('warn').hidden = s.conflict !== 'replace_original';
  $('ff-notice').hidden = !!s.ffmpeg_notice_seen;
  for (const id in SLIDERS) paintSlider(id, SLIDERS[id].get());
  if (document.activeElement !== $('tpl')) {
    $('tpl').value = s.name_template;
    tplPaint();
  }
  const c = document.querySelector(`input[name=corner][value=${s.panel.corner}]`);
  if (c) c.checked = true;
  $('screen').dataset.corner = s.panel.corner;
  $('toast-sw').checked = s.panel.windows_toast;
  $('icons-sw').checked = s.menu.icons;
  $('presets-sw').checked = s.menu.presets_enabled;
  $('merge-sw').checked = s.menu.merge;
  $('hist-sw').checked = s.history_enabled;
  $('upd-sw').checked = s.updates.check_on_start;
  $('pr-off').hidden = s.menu.presets_enabled;
  const sorts = $$('input[name=sort]');
  sorts.forEach((r, i) => {
    r.checked = r.value === s.menu.sort;
    if (r.checked) seg(r.closest('.seg'), i);
  });
  document.querySelectorAll('input[name=lang]').forEach((r, i) => {
    r.checked = r.value === s.language;
    if (r.checked) seg(r.closest('.seg'), i);
  });
  paintChips();
}

/* ---------- existing pieces ---------- */
async function pickFolder() {
  const d = T && T.dialog;
  if (!d) return showError(t('err.no_dialog'));
  try {
    const p = await d.open({ directory: true, multiple: false });
    if (p) return change({ output_dir: { kind: 'fixed', path: String(p) } }), true;
  } catch (e) {
    showError(t('err.pick', { err: e }));
  }
  return false;
}

// in-page confirm modal; resolves true/false. `k` names the strings: {title, text, yes} (default: replace-original)
function confirmModal(k = { title: 'modal.title', text: 'modal.text', yes: 'modal.yes' }, vars) {
  return new Promise((resolve) => {
    const m = $('modal'), yes = $('m-yes'), no = $('m-no');
    $('m-title').textContent = t(k.title, vars);
    $('m-text').textContent = t(k.text, vars);
    yes.textContent = t(k.yes);
    const prev = document.activeElement;
    const done = (v) => {
      m.hidden = true;
      m.classList.remove('open');
      document.removeEventListener('keydown', key);
      if (prev) prev.focus();
      resolve(v);
    };
    const key = (e) => {
      if (e.key === 'Escape') done(false);
      else if (e.key === 'Tab') { // two-button focus trap
        e.preventDefault();
        (document.activeElement === no ? yes : no).focus();
      }
    };
    yes.onclick = () => done(true);
    no.onclick = () => done(false);
    m.onclick = (e) => e.target === m && done(false);
    document.addEventListener('keydown', key);
    m.hidden = false;
    m.classList.add('open');
    no.focus();
  });
}

function toast(msg, isErr) {
  const t = $('toast');
  t.textContent = msg;
  t.classList.toggle('err', !!isErr);
  t.classList.remove('on');
  void t.offsetWidth; // restart the enter transition
  t.classList.add('on');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    t.classList.remove('on');
    toastTimer = setTimeout(() => (t.textContent = ''), 300);
  }, 5000);
}

async function menuAction(cmd, okKey, btn) {
  btn.disabled = true;
  btn.classList.add('busy');
  try {
    await invoke(cmd);
    toast(t(okKey));
  } catch (e) {
    toast(t('err.action', { err: e }), true);
  }
  btn.disabled = false;
  btn.classList.remove('busy');
}

function ffView(state, text, pct) {
  const sheet = $('ff-sheet'), bar = $('ff-bar');
  $('ff').dataset.state = state;
  $('ff-status').textContent = text;
  sheet.classList.toggle('working', state === 'downloading');
  sheet.classList.toggle('ok', state === 'ready');
  sheet.classList.toggle('err', state === 'error');
  const p = state === 'ready' ? 100 : state === 'downloading' ? pct : 0;
  sheet.style.setProperty('--p', p / 100);
  bar.hidden = state !== 'downloading';
  bar.style.setProperty('--p', p / 100);
  bar.setAttribute('aria-valuenow', Math.round(p));
  $('ff-pct').hidden = state !== 'downloading';
  $('ff-pct').textContent = t('pct', { n: Math.round(p) });
  $('ff-retry').hidden = state !== 'error';
  if (ffReady !== (state === 'ready')) {
    ffReady = state === 'ready';
    paintChips();
  }
}

async function refreshFfmpeg() {
  clearTimeout(ffTimer);
  try {
    const f = await invoke('ffmpeg_status');
    const pct = f.percent || 0;
    if (f.ready) ffView('ready', t('ff.ready'));
    else if (f.downloading) ffView('downloading', t('ff.downloading'), pct);
    else if (f.error) ffView('error', t('ff.failed', { err: f.error }));
    else ffView('checking', t('ff.checking'));
    if (!f.ready) ffTimer = setTimeout(refreshFfmpeg, f.downloading ? 1000 : 3000);
  } catch (e) {
    ffView('error', t('ff.status_err', { err: e }));
    $('ff-retry').hidden = true;
  }
}

function bind() {
  tabs.forEach((t, i) => (t.onclick = () => select(i)));
  $('tabs').onkeydown = tabKeys;
  document.addEventListener('click', (e) => {
    const g = e.target.closest('[data-goto]');
    if (g) select(tabs.indexOf($(g.dataset.goto)), true);
  });
  $('autostart').onchange = (e) => change({ autostart: e.target.checked });
  $$('input[name=lang]').forEach((r) => (r.onchange = () => { setLang(r.value); change({ language: r.value }); relabel(); }));
  $$('input[name=dir]').forEach((r) => {
    r.onchange = async () => {
      if (r.value === 'same_as_source') return change({ output_dir: { kind: 'same_as_source' } });
      if (s.output_dir.kind === 'fixed') return;
      // no folder yet: ask for one, fall back if cancelled
      if (!(await pickFolder())) render();
    };
  });
  $('pick').onclick = pickFolder;
  $('conflict').onchange = async (e) => {
    const v = e.target.value;
    if (v === 'replace_original' && !(await confirmModal())) return render();
    change({ conflict: v });
  };
  for (const id in SLIDERS) $(id).oninput = (e) => SLIDERS[id].set(+e.target.value);
  $('tpl').oninput = tplInput;
  $$('.token').forEach((b) => {
    b.onclick = () => {
      const f = $('tpl');
      f.focus();
      f.setRangeText(b.dataset.token, f.selectionStart, f.selectionEnd, 'end');
      tplInput();
    };
  });
  $$('input[name=corner]').forEach((r) => {
    r.onchange = () => {
      setIn('panel', 'corner', r.value);
    };
  });
  $('demo-play').onclick = demoPlay;
  $('toast-sw').onchange = (e) => setIn('panel', 'windows_toast', e.target.checked);
  $('icons-sw').onchange = (e) => setIn('menu', 'icons', e.target.checked);
  $('presets-sw').onchange = (e) => setIn('menu', 'presets_enabled', e.target.checked);
  $('merge-sw').onchange = (e) => setIn('menu', 'merge', e.target.checked);
  $('hist-sw').onchange = (e) => change({ history_enabled: e.target.checked });
  $('upd-sw').onchange = (e) => setIn('updates', 'check_on_start', e.target.checked);
  $$('input[name=sort]').forEach((r) => (r.onchange = () => setIn('menu', 'sort', r.value)));
  $('ff-notice-ok').onclick = () => change({ ffmpeg_notice_seen: true });
  $('ff-retry').onclick = async () => {
    try {
      await invoke('retry_ffmpeg_download');
    } catch (e) {
      showError(t('err.dl_start', { err: e }));
    }
    refreshFfmpeg();
  };
  $('menu-add').onclick = (e) => menuAction('install_context_menu', 'mn.added', e.currentTarget);
  $('menu-del').onclick = (e) => menuAction('remove_context_menu', 'mn.removed', e.currentTarget);
}

// Rewrites what the static markup does not cover after a language switch.
function relabel() {
  if (!s) return;
  render();
  if (catalog) {
    buildKinds($('targets'), catalog.targets, false);
    buildKinds($('sources'), catalog.sources, true);
    paintChips();
  }
  tplPaint();
  refreshFfmpeg();
  paintNav();
  if (typeof featuresRelabel === 'function') featuresRelabel();
}

// the backend tells every window when the language changed
if (T && T.event) {
  T.event.listen('language-changed', (e) => {
    if (!s || e.payload === s.language) return;
    s.language = e.payload === 'tr' ? 'tr' : 'en';
    setLang(s.language);
    relabel();
  });
}

// sections settle in each time the window comes back from the tray
function intro() {
  if (document.visibilityState !== 'visible') return;
  document.body.classList.remove('enter');
  void document.body.offsetWidth;
  document.body.classList.add('enter');
}
document.addEventListener('visibilitychange', intro);

// features.js loads after this file; DOMContentLoaded waits for both
document.addEventListener('DOMContentLoaded', async () => {
  bind();
  paintNav();
  intro();
  try {
    s = normalize(await invoke('get_settings'));
    setLang(s.language);
    ffReady = !!s.ffmpeg_ready;
    goodTpl = s.name_template;
    render();
    paintNav(); // header text in the saved language
  } catch (e) {
    showError(t('err.load', { err: e }));
    $$('main input,main select,main button').forEach((x) => (x.disabled = true));
    return;
  }
  await loadCatalog();
  refreshFfmpeg();
  if (typeof featuresInit === 'function') featuresInit();
});
