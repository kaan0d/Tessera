'use strict';
/* The chooser's custom page: a one-off set of options (format, size, quality, metadata, audio,
   largest file). Shares the globals of drop.js (opts, busy, el, card, go, status, choices, t). */
const FMT_Q = ['jpg', 'webp', 'avif']; // targets the quality slider means something for
let st = null; // get_settings (quality defaults)
let cs = null; // state of the form, kept while the same files are shown
let customOpen = false;

const customAvailable = () => !!opts && opts.count > 0 && opts.kinds.length === 1 && ['image', 'audio', 'video'].includes(opts.kinds[0]);

function buildCustomCard() {
  $('sec-custom').hidden = !customAvailable();
  $('dr-customcard').replaceChildren(card('', { kind: 'custom_open' }, 'i-sliders', t('dr.custom'), t('dr.custom_sub')));
}

const outKind = () => (cs.target === 'same' ? opts.kinds[0] : (opts.targets.find((x) => x.ext === cs.target) || {}).kind);
const showQuality = () => outKind() === 'image' && (cs.target === 'same' || FMT_Q.includes(cs.target));

function group(title, ...kids) {
  const g = el('div', 'cu-group');
  g.append(el('span', 'cu-label', title), ...kids);
  return g;
}
function pill(text, on, onclick) {
  const b = el('button', 'chip', text);
  b.type = 'button';
  b.setAttribute('aria-pressed', on);
  b.onclick = onclick;
  return b;
}
function fill(range) {
  range.style.setProperty('--fill', ((range.value - range.min) / (range.max - range.min)) * 100 + '%');
}
function sliderLine(id, min, max, value, fmtVal, onInput) {
  const r = el('input', 'slider');
  Object.assign(r, { type: 'range', id, min, max, step: 1, value });
  const out = el('output', 'cu-val', fmtVal(value));
  r.oninput = () => (fill(r), (out.textContent = fmtVal(+r.value)), onInput(+r.value));
  fill(r);
  const line = el('div', 'cu-line');
  line.append(r, out);
  return line;
}
function switchLine(id, text, key) {
  const lab = el('label', 'cu-switch');
  const sw = el('input', 'switch');
  Object.assign(sw, { type: 'checkbox', id, checked: cs[key] });
  sw.setAttribute('role', 'switch');
  sw.onchange = () => ((cs[key] = sw.checked), syncCustom());
  lab.append(el('span', null, text), sw);
  return lab;
}

function buildCustom() {
  const f = $('dr-custom');
  f.onsubmit = (e) => (e.preventDefault(), runCustom());
  // format
  const fmts = el('div', 'chips cu-chips');
  fmts.setAttribute('role', 'group');
  const list = [{ ext: 'same', label: t('dr.c_same') }, ...opts.targets.filter((x) => x.allowed).map((x) => ({ ext: x.ext, label: x.ext.toUpperCase() }))];
  fmts.append(...list.map((o) => {
    const b = pill(o.label, cs.target === o.ext, () => ((cs.target = o.ext), syncCustom()));
    b.dataset.ext = o.ext;
    return b;
  }));
  // size
  const modes = el('div', 'chips cu-chips');
  modes.setAttribute('role', 'group');
  for (const [m, key] of [['none', 'dr.c_original'], ['edge', 'dr.c_edge'], ['pct', 'dr.c_pct']]) {
    const b = pill(t(key), cs.resize === m, () => ((cs.resize = m), syncCustom()));
    b.dataset.mode = m;
    modes.append(b);
  }
  const num = el('input', 'field cu-num');
  Object.assign(num, { type: 'number', id: 'cu-resize-n', inputMode: 'numeric', step: 1 });
  num.setAttribute('aria-label', t('dr.c_resize'));
  num.oninput = () => ((cs.resize === 'edge' ? (cs.edge = +num.value) : (cs.pct = +num.value)), syncCustom());
  const numLine = el('div', 'cu-line cu-numline');
  numLine.append(num, el('span', 'cu-unit'));
  const gResize = group(t('dr.c_resize'), modes, numLine);
  gResize.id = 'cu-g-resize';
  // quality (images) and video quality
  const gQ = group(t('dr.c_quality'), sliderLine('cu-q', 1, 100, cs.quality, (v) => String(v), (v) => ((cs.quality = v), (cs.qTouched = true), syncCustom())));
  gQ.id = 'cu-g-q';
  const gM = group(t('dr.c_mbps'), sliderLine('cu-m', 1, 30, cs.mbps, (v) => t('dr.c_mbps_val', { n: v }), (v) => ((cs.mbps = v), (cs.mTouched = true), syncCustom())));
  gM.id = 'cu-g-m';
  // switches
  const gStrip = el('div', 'cu-group');
  gStrip.id = 'cu-g-strip';
  gStrip.append(switchLine('cu-strip', t('dr.c_strip'), 'strip'));
  const gAudio = el('div', 'cu-group');
  gAudio.id = 'cu-g-audio';
  gAudio.append(switchLine('cu-noaudio', t('dr.c_noaudio'), 'noAudio'));
  // largest size
  const size = el('input', 'field cu-num');
  Object.assign(size, { type: 'number', id: 'cu-size', min: 0, step: 'any', inputMode: 'decimal', value: cs.sizeMb });
  size.setAttribute('aria-label', t('dr.c_size'));
  size.oninput = () => ((cs.sizeMb = size.value), syncCustom());
  const sizeLine = el('div', 'cu-line cu-numline');
  sizeLine.append(size, el('span', 'cu-unit', 'MB'), el('span', 'cu-hint', t('dr.c_size_hint')));
  const need = el('p', 'dr-note cu-need', t('dr.c_need'));
  need.id = 'cu-need';
  need.setAttribute('role', 'status');
  f.replaceChildren(group(t('dr.c_format'), fmts), gResize, gQ, gM, gStrip, gAudio, group(t('dr.c_size'), sizeLine), need);
}

function customOptions() {
  const o = { no_audio: false, strip_metadata: false };
  const k = outKind();
  if (k === 'image' || k === 'video') {
    if (cs.resize === 'edge' && cs.edge >= 16) o.resize = { max_edge: Math.min(16384, Math.round(cs.edge)) };
    if (cs.resize === 'pct' && cs.pct >= 1) o.resize = { percent: Math.min(400, Math.round(cs.pct)) };
    o.strip_metadata = cs.strip;
  }
  if (showQuality() && cs.qTouched) o.jpeg_quality = cs.quality;
  if (k === 'video') {
    if (cs.mTouched) o.video_mbps = cs.mbps;
    o.no_audio = cs.noAudio;
  }
  const mb = parseFloat(cs.sizeMb);
  if (mb > 0) o.max_bytes = Math.max(1, Math.round(mb * 1048576));
  return o;
}
const hasChange = (o) => !!(o.resize || o.strip_metadata || o.no_audio || o.max_bytes);

function syncCustom() {
  const k = outKind();
  const vis = (id, on) => ($(id).hidden = !on);
  vis('cu-g-resize', k === 'image' || k === 'video');
  vis('cu-g-q', showQuality());
  vis('cu-g-m', k === 'video');
  vis('cu-g-strip', k === 'image' || k === 'video');
  vis('cu-g-audio', k === 'video');
  for (const b of $('dr-custom').querySelectorAll('[data-ext]')) b.setAttribute('aria-pressed', b.dataset.ext === cs.target);
  for (const b of $('dr-custom').querySelectorAll('[data-mode]')) b.setAttribute('aria-pressed', b.dataset.mode === cs.resize);
  const num = $('cu-resize-n');
  num.parentElement.hidden = cs.resize === 'none';
  if (cs.resize !== 'none' && document.activeElement !== num) {
    const edge = cs.resize === 'edge';
    num.min = edge ? 16 : 1;
    num.max = edge ? 16384 : 400;
    num.value = edge ? cs.edge : cs.pct;
    num.nextElementSibling.textContent = edge ? 'px' : '%';
  }
  const need = cs.target === 'same' && !hasChange(customOptions());
  $('cu-need').hidden = !need;
  $('dr-run').disabled = need || busy;
}

function showCustom(on) {
  customOpen = on;
  $('dr-main').hidden = on;
  $('dr-custom').hidden = !on;
  $('dr-back').hidden = !on;
  $('dr-run').hidden = !on;
  $('dr-cancel').hidden = on;
  $('dr-body').scrollTop = 0;
  status('');
  if (on) {
    if (!cs) {
      const q = (st && st.quality) || {};
      cs = { target: 'same', resize: 'none', edge: 1920, pct: 50, quality: q.jpeg || 80, qTouched: false, mbps: q.video_mbps || 8, mTouched: false, strip: false, noAudio: false, sizeMb: '' };
    }
    buildCustom();
    syncCustom();
    const first = $('dr-custom').querySelector('[aria-pressed=true]');
    if (first) first.focus({ preventScroll: true });
  } else {
    const c = document.querySelector('.dr-choice[data-choice*="custom_open"]');
    if (c) {
      choices().forEach((b) => (b.tabIndex = -1));
      c.tabIndex = 0;
      c.focus({ preventScroll: true });
    }
  }
}

function runCustom() {
  if (busy || !cs || $('dr-run').disabled) return;
  go({ kind: 'custom', target: cs.target, options: customOptions() }, $('dr-run'));
}

$('dr-back').onclick = () => showCustom(false);
$('dr-run').onclick = runCustom;
