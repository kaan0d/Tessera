'use strict';
const T = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const SVG = 'http://www.w3.org/2000/svg';
const invoke = (cmd, args) =>
  T && T.core ? T.core.invoke(cmd, args) : Promise.reject('Uygulama arka ucuna bağlanamadı.');

let s = null; // current settings object
let catalog = null; // {sources, targets}
let ffReady = false;
let saveTimer, savedTimer, toastTimer, ffTimer, hintTimer, lastField, goodTpl;

$('app-title').textContent = document.title;

// ponytail: fills only what an older settings.json may lack; the backend owns the real defaults
function normalize(x) {
  x.quality = { jpeg: 85, mp3_q: 2, ogg_q: 6, video_mbps: 8, gif_fps: 15, gif_width: 480, ...x.quality };
  x.panel = { corner: 'bottom_right', seconds: 7, windows_toast: false, ...x.panel };
  x.menu = { icons: true, sort: 'default', ...x.menu };
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

/* ---------- tabs ---------- */
const tabs = $$('[role=tab]');
let cur = 0;

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
  $('tabs').style.setProperty('--i', i);
  cur = i;
  scrollTo(0, 0);
  if (tabs[i].id !== 't-panel') demoStop();
}

function tabKeys(e) {
  const k = { ArrowRight: cur + 1, ArrowLeft: cur - 1, Home: 0, End: tabs.length - 1 }[e.key];
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
      $('saved-text').textContent = 'Kaydedildi';
      $('saved').classList.add('on');
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => $('saved').classList.remove('on'), 1600);
    } catch (e) {
      // a rejected template must not ride along with the next unrelated save
      if (f === 'tpl') tplMsg(String(e)), (s.name_template = goodTpl);
      else showError('Ayarlar kaydedilemedi: ' + e);
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
  'q-secs': { get: () => s.panel.seconds, set: (v) => setIn('panel', 'seconds', v), show: (v) => [v, 'sn'] },
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
const KINDS = [['image', 'Görsel', 'i-image'], ['audio', 'Ses', 'i-audio'], ['video', 'Video', 'i-film']];

function buildKinds(box, items, sources) {
  box.replaceChildren();
  for (const [kind, title, ic] of KINDS) {
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
      for (const [txt, all] of [['Tümü', true], ['Hiçbiri', false]]) {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'link';
        b.textContent = txt;
        b.setAttribute('aria-label', `${title}: ${txt.toLowerCase()}`);
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
      if (it.needs_ffmpeg) {
        c.dataset.ff = '';
        c.append(icon('i-download', 'ic badge'));
      }
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
    c.title = locked ? 'ffmpeg indirilince kullanılabilir' : 'ff' in c.dataset ? 'ffmpeg ile dönüştürülür' : '';
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
    hint(th, 'warn', [icon('i-alert'), 'Hiçbir biçim seçili değil. Sağ tık menüsünde dönüştürme seçeneği görünmez.']);
  } else if (anyFf && !ffReady) {
    hint(th, '', [icon('i-download', 'ic badge'), ' işaretli biçimler ffmpeg ister. İndirme bitince kendiliğinden açılır. ', gotoLink('Durumu gör', 't-ileri')]);
  } else if (anyFf) {
    hint(th, '', [icon('i-download', 'ic badge'), ' işaretli biçimler ffmpeg ile dönüştürülür.']);
  } else hint(th, '', []);
  th.hidden = !th.childNodes.length;
  const n = s.menu_sources.length;
  hint($('s-hint'), n ? '' : 'warn', n
    ? [`Menü ${n} dosya türünde görünür.`]
    : [icon('i-alert'), 'Hiçbir tür seçili değil. Sağ tık menüsü hiçbir dosyada görünmez.']);
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
  if (!ffReady && it.needs_ffmpeg) return paintHints(`${it.ext.toUpperCase()} için ffmpeg gerekiyor. İndirme bitince açılır.`);
  const on = s.enabled_targets.includes(it.ext);
  if (on && s.enabled_targets.length === 1) {
    chip.classList.remove('nudge');
    void chip.offsetWidth;
    chip.classList.add('nudge');
    return paintHints('En az bir biçim açık kalmalı.');
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
    $('cat-err').textContent = 'Biçim listesi alınamadı: ' + e;
  }
}

/* ---------- name template ---------- */
const TOKENS = ['{name}', '{format}', '{date}'];

function tplError(t) {
  if (!t.trim()) return 'Dosya adı boş olamaz.';
  if (/[\\/:*?"<>|]/.test(t)) return 'Şu karakterler kullanılamaz: \\ / : * ? " < > |';
  const bad = (t.match(/\{[^{}]*\}/g) || []).find((x) => !TOKENS.includes(x));
  if (bad) return `Bilinmeyen etiket: ${bad}. Kullanılabilenler: {name}, {format}, {date}.`;
  if (/[{}]/.test(t.replace(/\{[^{}]*\}/g, ''))) return 'Süslü parantezler eşleşmiyor.';
  return '';
}

function tplMsg(msg) {
  $('tpl-msg').textContent = msg;
  $('tpl').setAttribute('aria-invalid', !!msg);
  $('tpl-ex').classList.toggle('stale', !!msg);
}

function tplPaint() {
  const t = $('tpl').value;
  const err = tplError(t);
  tplMsg(err);
  if (err) return;
  const img = catalog ? catalog.targets.find((x) => x.kind === 'image' && x.ext !== 'png' && s.enabled_targets.includes(x.ext)) : null;
  const ext = img ? img.ext : 'jpg';
  const d = new Date();
  const date = [d.getFullYear(), d.getMonth() + 1, d.getDate()].map((x) => String(x).padStart(2, '0')).join('-');
  $('tpl-out').textContent = t.replaceAll('{name}', 'foto').replaceAll('{format}', ext).replaceAll('{date}', date) + '.' + ext;
}

function tplInput() {
  tplPaint();
  const t = $('tpl').value;
  if (!tplError(t) && t !== s.name_template) {
    s.name_template = t;
    save('tpl');
  }
}

/* ---------- panel preview ---------- */
let demoTimers = [];

function demoStop() {
  demoTimers.forEach(clearTimeout);
  demoTimers = [];
  $('demo').hidden = true;
}

function demoRow(li, p) {
  const sheet = li.querySelector('.sheet');
  sheet.style.setProperty('--p', p / 100);
  li.querySelector('.msg').textContent = p >= 100 ? 'Tamamlandı' : `%${Math.round(p)}`;
  if (p >= 100 && !li.classList.contains('is-ok')) {
    li.classList.add('is-ok');
    sheet.classList.replace('working', 'ok');
  }
}

function demoPlay() {
  demoStop();
  const d = $('demo'), rows = $$('#demo .job'), timer = $('demo-timer');
  rows.forEach((li) => {
    li.classList.remove('is-ok');
    li.querySelector('.sheet').classList.replace('ok', 'working');
    demoRow(li, 0);
  });
  $('demo-head').classList.remove('done');
  $('demo-title').textContent = 'Dönüştürülüyor';
  $('demo-sub').textContent = '2 dosya';
  timer.classList.remove('run');
  timer.style.transitionDuration = '';
  d.classList.remove('out');
  d.hidden = false;
  d.classList.remove('in');
  void d.offsetWidth;
  d.classList.add('in');
  let t = 0;
  const step = () => {
    t++;
    const a = Math.min(100, t * 8), b = Math.min(100, t * 5);
    demoRow(rows[0], a);
    demoRow(rows[1], b);
    $('demo-pct').textContent = Math.round((a + b) / 2);
    $('demo-bar').style.setProperty('--p', (a + b) / 200);
    $('demo-sub').textContent = b >= 100 ? '2 dosya hazır' : a >= 100 ? '1/2 dosya bitti' : '2 dosya';
    if (b < 100) return demoTimers.push(setTimeout(step, 130));
    $('demo-head').classList.add('done');
    $('demo-title').textContent = 'Tamamlandı';
    // the card waits for panel.seconds, drawn as a draining line
    const secs = s.panel.seconds;
    timer.style.transitionDuration = secs + 's';
    void timer.offsetWidth;
    timer.classList.add('run');
    demoTimers.push(setTimeout(() => d.classList.add('out'), secs * 1000));
    demoTimers.push(setTimeout(() => (d.hidden = true), secs * 1000 + 360));
  };
  demoTimers.push(setTimeout(step, 450));
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
  const sorts = $$('input[name=sort]');
  sorts.forEach((r, i) => {
    r.checked = r.value === s.menu.sort;
    if (r.checked) seg(r.closest('.seg'), i);
  });
  paintChips();
}

/* ---------- existing pieces ---------- */
async function pickFolder() {
  const d = T && T.dialog;
  if (!d) return showError('Klasör seçme penceresi kullanılamıyor.');
  try {
    const p = await d.open({ directory: true, multiple: false });
    if (p) return change({ output_dir: { kind: 'fixed', path: String(p) } }), true;
  } catch (e) {
    showError('Klasör seçilemedi: ' + e);
  }
  return false;
}

// in-page confirm modal; resolves true/false
function confirmModal() {
  return new Promise((resolve) => {
    const m = $('modal'), yes = $('m-yes'), no = $('m-no');
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

async function menuAction(cmd, okMsg, btn) {
  btn.disabled = true;
  btn.classList.add('busy');
  try {
    await invoke(cmd);
    toast(okMsg);
  } catch (e) {
    toast('İşlem başarısız: ' + e, true);
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
  $('ff-pct').textContent = `%${Math.round(p)}`;
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
    if (f.ready) ffView('ready', 'Hazır. Video ve ses dosyaları dönüştürülebilir.');
    else if (f.downloading) ffView('downloading', 'İndiriliyor, arka planda sürüyor.', pct);
    else if (f.error) ffView('error', `İndirilemedi: ${f.error}`);
    else ffView('checking', 'Denetleniyor, indirme birazdan başlar.');
    if (!f.ready) ffTimer = setTimeout(refreshFfmpeg, f.downloading ? 1000 : 3000);
  } catch (e) {
    ffView('error', 'ffmpeg durumu alınamadı: ' + e);
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
      if (!$('demo').hidden) demoPlay();
    };
  });
  $('demo-play').onclick = demoPlay;
  $('toast-sw').onchange = (e) => setIn('panel', 'windows_toast', e.target.checked);
  $('icons-sw').onchange = (e) => setIn('menu', 'icons', e.target.checked);
  $$('input[name=sort]').forEach((r) => (r.onchange = () => setIn('menu', 'sort', r.value)));
  $('ff-notice-ok').onclick = () => change({ ffmpeg_notice_seen: true });
  $('ff-retry').onclick = async () => {
    try {
      await invoke('retry_ffmpeg_download');
    } catch (e) {
      showError('İndirme başlatılamadı: ' + e);
    }
    refreshFfmpeg();
  };
  $('menu-add').onclick = (e) => menuAction('install_context_menu', 'Menü kaydedildi.', e.currentTarget);
  $('menu-del').onclick = (e) => menuAction('remove_context_menu', 'Menü kaldırıldı.', e.currentTarget);
}

// sections settle in each time the window comes back from the tray
function intro() {
  if (document.visibilityState !== 'visible') return demoStop();
  document.body.classList.remove('enter');
  void document.body.offsetWidth;
  document.body.classList.add('enter');
}
document.addEventListener('visibilitychange', intro);

(async () => {
  bind();
  intro();
  try {
    s = normalize(await invoke('get_settings'));
    ffReady = !!s.ffmpeg_ready;
    goodTpl = s.name_template;
    render();
  } catch (e) {
    showError('Ayarlar yüklenemedi: ' + e);
    $$('main input,main select,main button').forEach((x) => (x.disabled = true));
    return;
  }
  loadCatalog();
  refreshFfmpeg();
})();
