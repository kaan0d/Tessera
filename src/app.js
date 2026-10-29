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
  x.language = langOf(x.language);
  x.quality = { jpeg: 85, webp: 80, webp_lossless: false, mp3_q: 2, ogg_q: 6, video_mbps: 8, gif_fps: 15, gif_width: 480, ...x.quality };
  x.panel = { corner: 'top_right', seconds: 7, windows_toast: false, ...x.panel };
  x.menu = { icons: true, sort: 'default', presets_enabled: true, merge: true, pdf_tools: true, folder: true, folder_recursive: false, sendto: false, ...x.menu };
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
  'q-webp': { get: () => s.quality.webp, set: (v) => setIn('quality', 'webp', v), show: (v) => [v, ''] },
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
    const pic = document.createElement('img');
    pic.src = `kinds/${kind}.svg`;
    pic.alt = '';
    tile.append(pic);
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
        c.append(ffTag());
      }
      if (!sources && it.needs_office) c.dataset.lo = '';
      if (sources) c.title = srcNeedsFf(it) ? t('chip.src_ff') : RAW.includes(it.ext) ? t('chip.raw') : '';
      c.onclick = () => (sources ? toggleSource(it.ext) : toggleTarget(it, c));
      chips.append(c);
    }
    grow.append(line, chips);
    row.append(tile, grow);
    box.append(row);
  }
}

// null = every catalog entry
const PROFILES = {
  normal: {
    src: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif', 'heic', 'heif', 'pdf', 'wav', 'mp3', 'm4a', 'mp4', 'mov', 'docx', 'xlsx', 'pptx'],
    dst: ['png', 'jpg', 'webp', 'gif', 'pdf', 'wav', 'mp3', 'mp4', 'docx'],
  },
  all: { src: null, dst: null },
};
const profileSets = (p) => ({
  menu_sources: catalog.sources.map((x) => x.ext).filter((e) => !p.src || p.src.includes(e)),
  enabled_targets: catalog.targets.map((x) => x.ext).filter((e) => !p.dst || p.dst.includes(e)),
});
const sameList = (a, b) => a.length === b.length && a.every((e) => b.includes(e));
const activeProfile = () => Object.keys(PROFILES).find((id) => {
  const p = profileSets(PROFILES[id]);
  return sameList(p.menu_sources, s.menu_sources) && sameList(p.enabled_targets, s.enabled_targets);
});

function paintProfiles() {
  const box = $('profiles');
  if (!box.firstChild) {
    for (const id of [...Object.keys(PROFILES), 'custom']) {
      const c = document.createElement('button');
      c.type = 'button';
      c.className = 'profile';
      c.dataset.id = id;
      const name = document.createElement('span'), desc = document.createElement('span');
      name.className = 'p-name';
      desc.className = 'p-desc';
      c.append(name, desc, icon('i-check', 'ic p-check'));
      if (id !== 'custom') c.onclick = () => change(profileSets(PROFILES[id]));
      box.append(c);
    }
  }
  const cur = activeProfile() || 'custom';
  for (const c of box.children) {
    c.firstChild.textContent = t('profile.' + c.dataset.id);
    c.children[1].textContent = t(c.dataset.id === 'custom' ? 'profile.custom_card' : `profile.${c.dataset.id}_desc`);
    c.setAttribute('aria-pressed', c.dataset.id === cur);
  }
  // always shown with a line of the same size, so picking a chip never pushes the page down
  $('p-hint').textContent = t(cur !== 'custom' ? 'profile.matched' : 'profile.custom_desc');
  const fine = document.createElement('button');
  fine.type = 'button';
  fine.className = 'link inline';
  fine.textContent = t('fmt.fine_tune');
  fine.onclick = () => {
    const g = $('g-sources');
    g.open = true;
    g.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  $('p-hint').append(fine);
}

function paintChips() {
  if (!catalog || !s) return;
  for (const c of $$('#targets .chip')) {
    const on = s.enabled_targets.includes(c.dataset.ext);
    const locked = 'ff' in c.dataset && !ffReady;
    c.setAttribute('aria-pressed', on);
    c.setAttribute('aria-disabled', locked);
    c.title = locked ? t('chip.locked') : 'ff' in c.dataset ? t('chip.ff') : 'lo' in c.dataset ? t('chip.lo') : '';
  }
  for (const c of $$('#sources .chip')) c.setAttribute('aria-pressed', s.menu_sources.includes(c.dataset.ext));
  paintProfiles();
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
    flashing = true;
    hintTimer = setTimeout(() => { flashing = false; paintHints(); }, 3200);
  } else if (!s.enabled_targets.length) {
    hint(th, 'warn', [icon('i-alert'), t('hint.none_targets')]);
  } else if (anyFf && !ffReady) {
    const wait = [t('hint.ff_wait'), gotoLink(t('hint.see_status'), 't-ileri')];
    if (ffPct !== null) {
      const bar = document.createElement('span');
      bar.className = 'hint-bar';
      bar.setAttribute('role', 'progressbar');
      bar.setAttribute('aria-label', t('ff.bar'));
      bar.setAttribute('aria-valuemin', 0);
      bar.setAttribute('aria-valuemax', 100);
      bar.setAttribute('aria-valuenow', ffPct);
      bar.style.setProperty('--p', ffPct / 100);
      bar.append(document.createElement('span'));
      const pct = document.createElement('b');
      pct.textContent = t('pct', { n: ffPct });
      wait.push(bar, pct);
    }
    hint(th, '', wait);
  } else if (anyFf) {
    hint(th, '', [t('hint.ff_ready')]);
  } else hint(th, '', []);
  th.hidden = !th.childNodes.length;
  const n = s.menu_sources.length;
  $('s-count').textContent = t('fmt.src_count', { n });
  const raw = s.menu_sources.some((e) => RAW.includes(e));
  hint($('s-hint'), n ? '' : 'warn', n
    ? [t('hint.sources', { n }), ' ', t('hint.src_ff'), ...(raw ? [' ' + t('hint.raw')] : [])]
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
    paintWebp();
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
  $('ff-notice').hidden = !!s.ffmpeg_notice_seen || !!s.ffmpeg_ready;
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
  $('pdftools-sw').checked = s.menu.pdf_tools;
  $('folder-sw').checked = s.menu.folder;
  $('folder-rec-sw').checked = s.menu.folder_recursive;
  $('folder-rec-sw').disabled = !s.menu.folder;
  $('folder-rec-sw').closest('.row').classList.toggle('off', !s.menu.folder);
  $('sendto-sw').checked = s.menu.sendto;
  paintWebp();
  $('hist-sw').checked = s.history_enabled;
  $('upd-sw').checked = s.updates.check_on_start;
  $('pr-off').hidden = s.menu.presets_enabled;
  const sorts = $$('input[name=sort]');
  sorts.forEach((r, i) => {
    r.checked = r.value === s.menu.sort;
    if (r.checked) seg(r.closest('.seg'), i);
  });
  if (!$('lang').options.length) {
    // each language in its own name and script
    for (const [code, name] of Object.entries(LANGS)) {
      const o = new Option(name, code);
      o.lang = code;
      $('lang').add(o);
    }
  }
  $('lang').value = s.language;
  paintChips();
}

// lossy WebP is encoded by ffmpeg; until it is ready (or with lossless on) the slider has nothing to say
function paintWebp() {
  const ll = s.quality.webp_lossless;
  $('webp-ll').checked = ll;
  $('q-webp').disabled = ll;
  $('webp-q').classList.toggle('off', ll);
  const wp = catalog && catalog.targets.find((x) => x.ext === 'webp');
  $('webp-note').hidden = ll || ffReady || !(wp && wp.better_with_ffmpeg);
}

/* ---------- files dragged onto the window ---------- */
// Tauri reports the drag (enter/over/leave/drop with real paths); the chooser window takes it from there
function dragDrop() {
  const w = T && T.webview && T.webview.getCurrentWebview && T.webview.getCurrentWebview();
  if (!w) return;
  const dz = $('dz');
  let files = false;
  w.onDragDropEvent(({ payload: p }) => {
    if (p.type === 'enter') files = !!(p.paths && p.paths.length);
    if (p.type === 'enter' || p.type === 'over') return dz.classList.toggle('on', files);
    dz.classList.remove('on');
    if (p.type === 'drop' && p.paths && p.paths.length)
      invoke('drop_files', { paths: p.paths }).catch((e) => showError(t('err.drop', { err: e })));
  });
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

let flashing = false; // a flash message owns the targets hint
let ffPct = null; // percent while downloading, else null

function ffView(state, text, pct) {
  const bar = $('ff-bar');
  $('ff').dataset.state = state;
  $('ff-status').textContent = text;
  const p = state === 'ready' ? 100 : state === 'downloading' ? pct : 0;
  bar.hidden = state !== 'downloading';
  bar.style.setProperty('--p', p / 100);
  bar.setAttribute('aria-valuenow', Math.round(p));
  $('ff-pct').hidden = state !== 'downloading';
  $('ff-pct').textContent = t('pct', { n: Math.round(p) });
  $('ff-retry').hidden = state !== 'error' && state !== 'later' && state !== 'ready';
  $('ff-del').hidden = state !== 'ready';
  $('ff-retry').querySelector('span').textContent = t(state === 'later' ? 'ff.download_now' : state === 'ready' ? 'ff.update' : 'ff.retry');
  const dl = state === 'downloading' ? Math.round(pct) : null;
  if (dl !== ffPct) {
    ffPct = dl;
    if (catalog && s && !flashing) paintHints();
  }
  if (ffReady !== (state === 'ready')) {
    ffReady = state === 'ready';
    paintChips();
    if (s) paintWebp();
  }
}

async function refreshFfmpeg() {
  clearTimeout(ffTimer);
  try {
    const f = await invoke('ffmpeg_status');
    const pct = f.percent || 0;
    if (f.downloading) ffView('downloading', t('ff.downloading'), pct);
    else if (f.ready) ffView('ready', t('ff.ready'));
    else if (f.error) ffView('error', t('ff.failed', { err: f.error }));
    else if (f.later) ffView('later', t('ff.later'));
    else ffView('checking', t('ff.checking'));
    if (!f.ready || f.downloading) ffTimer = setTimeout(refreshFfmpeg, f.downloading ? 1000 : 3000);
  } catch (e) {
    ffView('error', t('ff.status_err', { err: e }));
    $('ff-retry').hidden = true;
  }
}

let loTimer;

// Document support (LibreOffice): downloaded only when the user presses the button.
async function refreshOffice() {
  clearTimeout(loTimer);
  let f;
  try {
    f = await invoke('office_status');
  } catch (e) {
    $('lo-status').textContent = t('lo.status_err', { err: e });
    return;
  }
  const state = f.downloading ? 'downloading' : f.ready ? (f.outdated ? 'outdated' : 'ready') : f.error ? 'error' : 'missing';
  $('lo').dataset.state = state;
  const pct = Math.round(f.percent || 0);
  $('lo-status').textContent = state === 'downloading' ? t('lo.downloading')
    : state === 'ready' ? t(f.source === 'system' ? 'lo.system' : 'lo.ready')
    : state === 'outdated' ? t('lo.outdated')
    : state === 'error' ? t('ff.failed', { err: f.error })
    : t('lo.missing');
  $('lo-bar').hidden = $('lo-pct').hidden = state !== 'downloading';
  $('lo-bar').style.setProperty('--p', pct / 100);
  $('lo-bar').setAttribute('aria-valuenow', pct);
  $('lo-pct').textContent = t('pct', { n: pct });
  // our own copy can be fetched again (like ffmpeg); an installed LibreOffice updates itself
  $('lo-get').hidden = state === 'downloading' || (state === 'ready' && f.source !== 'own');
  $('lo-get').querySelector('span').textContent = t(state === 'error' ? 'ff.retry' : state === 'outdated' || state === 'ready' ? 'lo.update' : 'lo.get');
  $('lo-del').hidden = !((state === 'ready' || state === 'outdated') && f.source === 'own');
  if (f.downloading) loTimer = setTimeout(refreshOffice, 1000);
}

function bind() {
  tabs.forEach((t, i) => (t.onclick = () => select(i)));
  $('tabs').onkeydown = tabKeys;
  document.addEventListener('click', (e) => {
    const g = e.target.closest('[data-goto]');
    if (g) select(tabs.indexOf($(g.dataset.goto)), true);
  });
  $('autostart').onchange = (e) => change({ autostart: e.target.checked });
  $('lang').onchange = (e) => { setLang(e.target.value); change({ language: LANG }); relabel(); };
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
  $('pdftools-sw').onchange = (e) => setIn('menu', 'pdf_tools', e.target.checked);
  $('folder-sw').onchange = (e) => setIn('menu', 'folder', e.target.checked);
  $('folder-rec-sw').onchange = (e) => setIn('menu', 'folder_recursive', e.target.checked);
  $('sendto-sw').onchange = (e) => setIn('menu', 'sendto', e.target.checked);
  $('webp-ll').onchange = (e) => setIn('quality', 'webp_lossless', e.target.checked);
  $('hist-sw').onchange = (e) => change({ history_enabled: e.target.checked });
  $('upd-sw').onchange = (e) => setIn('updates', 'check_on_start', e.target.checked);
  $$('input[name=sort]').forEach((r) => (r.onchange = () => setIn('menu', 'sort', r.value)));
  $('ff-notice-ok').onclick = async () => {
    $('ff-notice').hidden = true;
    try {
      await invoke('retry_ffmpeg_download');
      if (s) { s.ffmpeg_notice_seen = true; s.ffmpeg_later = false; }
    } catch (e) {
      showError(t('err.dl_start', { err: e }));
    }
    refreshFfmpeg();
  };
  $('ff-notice-later').onclick = async () => {
    $('ff-notice').hidden = true;
    await change({ ffmpeg_notice_seen: true, ffmpeg_later: true });
    refreshFfmpeg();
  };
  $('ff-retry').onclick = async () => {
    try {
      await invoke('retry_ffmpeg_download');
      if (s) { s.ffmpeg_notice_seen = true; s.ffmpeg_later = false; }
      $('ff-notice').hidden = true;
    } catch (e) {
      showError(t('err.dl_start', { err: e }));
    }
    refreshFfmpeg();
  };
  $('lo-get').onclick = async () => {
    try {
      await invoke('office_download');
    } catch (e) {
      showError(t('err.dl_start', { err: e }));
    }
    refreshOffice();
  };
  $('ff-del').onclick = async (e) => {
    e.currentTarget.disabled = true;
    try {
      await invoke('ffmpeg_remove');
      if (s) { s.ffmpeg_later = true; s.ffmpeg_notice_seen = true; }
      toast(t('ff.removed'));
    } catch (err) {
      toast(t('err.action', { err }), true);
    }
    e.currentTarget.disabled = false;
    refreshFfmpeg();
  };
  $('lo-del').onclick = async (e) => {
    e.currentTarget.disabled = true;
    try {
      await invoke('office_remove');
      toast(t('lo.removed'));
    } catch (err) {
      toast(t('err.action', { err }), true);
    }
    e.currentTarget.disabled = false;
    refreshOffice();
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
  refreshOffice();
  paintNav();
  if (typeof featuresRelabel === 'function') featuresRelabel();
}

// the backend tells every window when the language changed
if (T && T.event) {
  T.event.listen('language-changed', (e) => {
    if (!s || e.payload === s.language) return;
    s.language = langOf(e.payload);
    setLang(s.language);
    relabel();
  });
  // a download finished or document support was removed
  T.event.listen('office-changed', () => refreshOffice());
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
  initWindowChrome();
  bind();
  dragDrop();
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
    const why = t('err.load', { err: e });
    showError(`${why}${/[.!?]$/.test(why) ? '' : '.'} ${t('err.load_dim')}`);
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'btn sm';
    retry.textContent = t('err.retry');
    retry.onclick = () => location.reload();
    $('error').append(retry);
    const where = document.createElement('button');
    where.type = 'button';
    where.className = 'btn sm';
    where.textContent = t('err.open_folder');
    where.onclick = () => invoke('open_settings_folder').catch(() => {});
    $('error').append(where);
    $$('main input,main select,main button:not(#error .btn)').forEach((x) => (x.disabled = true));
    return;
  }
  await loadCatalog();
  refreshFfmpeg();
  refreshOffice();
  if (typeof featuresInit === 'function') featuresInit();
});
