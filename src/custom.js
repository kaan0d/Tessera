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
const srcKind = () => opts.kinds[0];
const isGif = () => cs.target === 'gif' && srcKind() === 'video';
const hasSound = () => ['audio', 'video'].includes(outKind()) && !['wav', 'flac'].includes(cs.target);
// "90", "1:30" or "1:02:03.5" in seconds; null when empty or not a time
function parseTime(v) {
  const m = /^\s*(?:(?:(\d+):)?(\d+):)?(\d+(?:[.,]\d+)?)\s*$/.exec(v);
  return m ? (+(m[1] || 0)) * 3600 + (+(m[2] || 0)) * 60 + parseFloat(m[3].replace(',', '.')) : null;
}
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
  // sound
  const gKbps = group(t('dr.c_kbps'), sliderLine('cu-k', 32, 320, cs.kbps, (v) => t('dr.c_kbps_val', { n: v }), (v) => ((cs.kbps = v), (cs.kTouched = true), syncCustom())));
  gKbps.id = 'cu-g-k';
  const gMono = el('div', 'cu-group');
  gMono.id = 'cu-g-mono';
  gMono.append(switchLine('cu-mono', t('dr.c_mono'), 'mono'));
  // GIF
  const gFps = group(t('dr.c_fps'), sliderLine('cu-fps', 5, 30, cs.fps, (v) => t('dr.c_fps_val', { n: v }), (v) => ((cs.fps = v), (cs.fTouched = true), syncCustom())));
  gFps.id = 'cu-g-fps';
  const gW = group(t('dr.c_gifw'), sliderLine('cu-gw', 120, 1280, cs.gifW, (v) => v + ' px', (v) => ((cs.gifW = v), (cs.wTouched = true), syncCustom())));
  gW.id = 'cu-g-gw';
  // cut
  const timeField = (id, key, label) => {
    const i = el('input', 'field cu-num');
    Object.assign(i, { type: 'text', id, inputMode: 'decimal', value: cs[key], placeholder: label });
    i.setAttribute('aria-label', label);
    i.oninput = () => ((cs[key] = i.value), syncCustom());
    return i;
  };
  const cutLine = el('div', 'cu-line cu-numline');
  cutLine.append(timeField('cu-t0', 'tStart', t('dr.c_from')), el('span', 'cu-unit', '-'), timeField('cu-t1', 'tEnd', t('dr.c_to')));
  const gCut = group(t('dr.c_cut'), cutLine, el('p', 'cu-hint', t('dr.c_cut_hint')));
  gCut.id = 'cu-g-cut';
  // largest size
  const size = el('input', 'field cu-num');
  Object.assign(size, { type: 'number', id: 'cu-size', min: 0, step: 'any', inputMode: 'decimal', value: cs.sizeMb, placeholder: t('dr.c_size_ph') });
  size.setAttribute('aria-label', t('dr.c_size'));
  size.oninput = () => ((cs.sizeMb = size.value), syncCustom());
  const sizeLine = el('div', 'cu-line cu-numline');
  sizeLine.append(size, el('span', 'cu-unit', 'MB'));
  const need = el('p', 'dr-note cu-need', t('dr.c_need'));
  need.id = 'cu-need';
  need.setAttribute('role', 'status');
  // keep these settings as a menu entry
  const pname = el('input', 'field plain');
  Object.assign(pname, { type: 'text', id: 'cu-pname', maxLength: 40, placeholder: t('dr.c_pname') });
  pname.setAttribute('aria-label', t('dr.c_pname'));
  const psave = el('button', 'btn sm ghost', t('dr.c_psave'));
  psave.type = 'button';
  psave.id = 'cu-psave';
  psave.onclick = savePreset;
  const pline = el('div', 'cu-line');
  pline.append(pname, psave);
  f.replaceChildren(group(t('dr.c_format'), fmts), gResize, gQ, gM, gFps, gW, gKbps, gMono, gCut, gStrip, gAudio, group(t('dr.c_size'), sizeLine), need, group(t('dr.c_preset'), pline));
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
  if (['audio', 'video'].includes(srcKind())) {
    const a = parseTime(cs.tStart), b = parseTime(cs.tEnd);
    if (a > 0) o.trim_start = a;
    if (b > 0) o.trim_end = b;
  }
  if (hasSound()) {
    if (cs.kTouched) o.audio_kbps = cs.kbps;
    o.mono = cs.mono;
  }
  if (isGif()) {
    if (cs.fTouched) o.gif_fps = cs.fps;
    if (cs.wTouched) o.gif_width = cs.gifW;
  }
  const mb = parseFloat(cs.sizeMb);
  if (mb > 0) o.max_bytes = Math.max(1, Math.round(mb * 1048576));
  return o;
}
const hasChange = (o) => !!(o.resize || o.strip_metadata || o.no_audio || o.max_bytes || o.audio_kbps || o.mono || o.trim_start || o.trim_end);
// text that is no time, or a cut that ends before it starts
function cutError() {
  const a = parseTime(cs.tStart), b = parseTime(cs.tEnd);
  if ((cs.tStart.trim() && a === null) || (cs.tEnd.trim() && b === null)) return true;
  return b !== null && b <= (a || 0);
}

function syncCustom() {
  const k = outKind();
  const vis = (id, on) => ($(id).hidden = !on);
  vis('cu-g-resize', k === 'image' || k === 'video');
  vis('cu-g-q', showQuality());
  vis('cu-g-m', k === 'video');
  vis('cu-g-strip', k === 'image' || k === 'video');
  vis('cu-g-audio', k === 'video');
  vis('cu-g-k', hasSound());
  vis('cu-g-mono', hasSound());
  vis('cu-g-cut', ['audio', 'video'].includes(srcKind()));
  vis('cu-g-fps', isGif());
  vis('cu-g-gw', isGif());
  $('cu-g-cut').querySelector('.cu-hint').classList.toggle('err', cutError());
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
  $('dr-run').disabled = need || busy || cutError();
  $('cu-psave').disabled = busy || cutError();
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
      cs = { target: 'same', resize: 'none', edge: 1920, pct: 50, quality: q.jpeg || 80, qTouched: false, mbps: q.video_mbps || 8, mTouched: false, strip: false, noAudio: false, sizeMb: '', kbps: 128, kTouched: false, mono: false, fps: q.gif_fps || 12, fTouched: false, gifW: q.gif_width || 480, wTouched: false, tStart: '', tEnd: '' };
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

async function savePreset() {
  const name = $('cu-pname').value.trim();
  if (!name) return ($('cu-pname').focus(), status(t('dr.c_pname_need'), true));
  try {
    await invoke('save_preset', { preset: { id: '', custom_name: name, kinds: opts.kinds, target: cs.target, options: customOptions() } });
    status(t('dr.c_psaved'));
  } catch (e) {
    status(String(e), true);
  }
}

function runCustom() {
  if (busy || !cs || $('dr-run').disabled) return;
  go({ kind: 'custom', target: cs.target, options: customOptions() }, $('dr-run'));
}

$('dr-back').onclick = () => showCustom(false);
$('dr-run').onclick = runCustom;
