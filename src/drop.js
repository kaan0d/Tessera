'use strict';
/* The chooser: files dropped on the exe, sent through Send to or dragged onto the settings window.
   It shows them as a small stack of sheets and asks what to make of them. */
const T = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const SVG = 'http://www.w3.org/2000/svg';
const invoke = (cmd, args) => (T && T.core ? T.core.invoke(cmd, args) : Promise.reject(t('err.backend')));

const KINDS = ['image', 'audio', 'video', 'document'];
const KIND_ICON = { image: 'i-image', audio: 'i-audio', video: 'i-film', document: 'i-doc' };
// pictograms inside the 25x30 sheet, as on the progress card
const SHEET = 'M4 1h11.5L23 8.5V27a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V3a2 2 0 0 1 2-2z';
const PICT = {
  image: 'M6.5 23.5l4.5-5 3.5 3.5 2-2 2.5 3.5M9.5 14.6v.1',
  video: 'M10 15v8l6.5-4z',
  audio: 'M11 23v-8.5l6-1.5v8M11 23a1.7 1.7 0 1 1-1.7-1.7A1.7 1.7 0 0 1 11 23zM17 21a1.7 1.7 0 1 1-1.7-1.7A1.7 1.7 0 0 1 17 21z',
  document: 'M7 15h11M7 19h11M7 23h7',
};

let files = []; // paths as the backend sent them
let opts = null; // get_drop_options
let ffReady = false;
let busy = false;
let seq = 0; // drops can overlap; only the newest answer is drawn

const fileName = (p) => String(p).split(/[\\/]/).pop();
setLang('en');

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
function icon(id, cls) {
  const i = svg('svg', { class: cls || 'ic', 'aria-hidden': 'true' });
  i.append(svg('use', { href: '#' + id }));
  return i;
}

/* ---------- the stack: up to three sheets, the front one fills once a choice runs ---------- */
function sheet(kind, i) {
  const id = 'dclip' + i;
  const s = svg('svg', { class: 'sheet', viewBox: '0 0 25 30' });
  const defs = svg('defs', {});
  const clip = svg('clipPath', { id });
  clip.append(svg('path', { d: SHEET }));
  defs.append(clip);
  const g = svg('g', { 'clip-path': `url(#${id})` });
  const level = svg('g', { class: 'level' });
  level.append(svg('path', { class: 'ink', d: 'M-24 0q3-2 6 0t6 0t6 0t6 0t6 0t6 0t6 0t6 0V40h-48z' }));
  g.append(level);
  s.append(defs, svg('rect', { class: 'paper', x: 2, y: 1, width: 21, height: 28, rx: 2 }), g,
    svg('path', { class: 'outline', d: SHEET }), svg('path', { class: 'fold', d: 'M15.5 1v5.5a2 2 0 0 0 2 2H23' }),
    svg('path', { class: 'pict', d: PICT[kind] || PICT.document }));
  return s;
}

function paintStack(kinds, n) {
  const box = $('dr-stack');
  const shown = Math.min(Math.max(n, 1), 3);
  // front sheet first: with one kind every sheet shows it, mixed drops show each kind once
  const list = Array.from({ length: shown }, (_, i) => kinds[i % Math.max(kinds.length, 1)] || 'document').reverse();
  box.replaceChildren(...list.map((k, i) => {
    const s = sheet(k, i);
    s.style.setProperty('--k', shown - 1 - i);
    s.classList.add('s' + (shown - 1 - i));
    return s;
  }));
  box.dataset.n = shown;
  if (n > 1) box.append(el('span', 'dr-count', n > 999 ? '999+' : String(n)));
}

/* ---------- choices ---------- */
function choiceBtn(cls, choice, label) {
  const b = el('button', cls);
  b.type = 'button';
  b.dataset.choice = JSON.stringify(choice);
  if (label) b.setAttribute('aria-label', label);
  return b;
}

function lockReason(x) {
  if (x.needs_ffmpeg && !ffReady) return 'ff';
  return x.needs_office ? 'lo' : 'off';
}

function buildTargets() {
  const box = $('dr-targets');
  box.replaceChildren();
  const list = opts.targets;
  $('sec-convert').hidden = !list.length && opts.kinds.length < 2;
  let ff = false, lo = false, off = false;
  for (const kind of KINDS) {
    const items = list.filter((x) => x.kind === kind);
    if (!items.length) continue;
    const row = el('div', 'dr-kind');
    const lab = el('span', 'dr-kind-label');
    lab.id = 'k-' + kind;
    lab.append(icon(KIND_ICON[kind]), t('kind.' + kind));
    const chips = el('div', 'chips');
    chips.setAttribute('role', 'group');
    chips.setAttribute('aria-labelledby', lab.id);
    for (const x of items) {
      const c = choiceBtn('chip dr-choice', { kind: 'target', target: x.ext }, t('dr.to', { fmt: x.ext.toUpperCase() }));
      c.append(x.ext.toUpperCase());
      if (x.needs_ffmpeg) c.append(ffTag());
      if (!x.allowed) {
        const why = lockReason(x);
        if (why === 'ff') ff = true;
        else if (why === 'lo') lo = true;
        else off = true;
        c.setAttribute('aria-disabled', 'true');
        c.dataset.why = why;
        c.title = t({ ff: 'dr.locked_ff', lo: 'dr.locked_lo', off: 'dr.locked_off' }[why], { fmt: x.ext.toUpperCase() });
      } else if (x.needs_ffmpeg) c.title = t('chip.ff');
      chips.append(c);
    }
    row.append(lab, chips);
    box.append(row);
  }
  const note = $('dr-tnote');
  const parts = [];
  if (!list.length) parts.push(t('dr.no_common'));
  if (ff) parts.push(t('dr.note_ff'));
  if (lo) parts.push(t('dr.note_lo'));
  if (off) parts.push(t('dr.note_off'));
  note.hidden = !parts.length;
  note.textContent = parts.join(' ');
  if (ff) {
    // the locked formats are a dead end without a way to unlock them from here
    const get = el('button', 'link inline');
    get.type = 'button';
    get.textContent = t('dr.get_ff');
    get.onclick = async () => {
      get.disabled = true;
      try {
        await invoke('retry_ffmpeg_download');
        get.textContent = t('dr.ff_started');
      } catch (e) {
        get.disabled = false;
      }
    };
    note.append(' ', get);
  }
  if (lo) {
    const get = el('button', 'link inline');
    get.type = 'button';
    get.textContent = t('dr.get_lo');
    get.onclick = async () => {
      get.disabled = true;
      try {
        await invoke('office_download');
        get.textContent = t('dr.lo_started');
      } catch (e) {
        get.disabled = false;
      }
    };
    note.append(' ', get);
  }
}

function card(cls, choice, ic, name, sub) {
  const b = choiceBtn('dr-card dr-choice ' + cls, choice);
  const tile = el('span', 'tile sm');
  tile.append(icon(ic));
  const txt = el('span', 'grow');
  txt.append(el('span', 'dr-card-name', name), el('span', 'dr-card-sub', sub));
  b.append(tile, txt, icon('i-arrow', 'ic go'));
  return b;
}

function buildPresets() {
  $('sec-presets').hidden = !opts.presets.length;
  $('dr-presets').replaceChildren(...opts.presets.map((p) => card('', { kind: 'preset', id: p.id }, 'i-wand', p.name, presetSummary(p))));
}

function buildMerge() {
  $('sec-merge').hidden = !opts.merge.length;
  $('dr-merge').replaceChildren(...opts.merge.map((m) => card('', { kind: 'merge', merge: m }, m === 'pdf' ? 'i-doc' : 'i-audio',
    t(m === 'pdf' ? 'dr.merge_pdf' : 'dr.merge_audio'), t(m === 'pdf' ? 'dr.merge_pdf_sub' : 'dr.merge_audio_sub', { n: opts.count }))));
}

const PDF_CARDS = [
  [{ tool: 'split' }, 'i-doc', 'dr.pdf_split', 'dr.pdf_split_sub'],
  [{ tool: 'compress', strong: false }, 'i-download', 'dr.pdf_compress', 'dr.pdf_compress_sub'],
  [{ tool: 'compress', strong: true }, 'i-download', 'dr.pdf_compress_strong', 'dr.pdf_compress_strong_sub'],
];
const PDF_TURNS = [[90, 'dr.pdf_right'], [270, 'dr.pdf_left'], [180, 'dr.pdf_180']];

function buildPdf() {
  $('sec-pdf').hidden = !opts.pdf_tools;
  if (!opts.pdf_tools) return;
  $('dr-pdf').replaceChildren(...PDF_CARDS.map(([pdf, ic, name, sub]) => card('', { kind: 'pdf', pdf }, ic, t(name), t(sub, { n: opts.count }))));
  $('dr-rotate').replaceChildren(...PDF_TURNS.map(([angle, key]) => {
    const c = choiceBtn('chip dr-choice', { kind: 'pdf', pdf: { tool: 'rotate', angle, pages: '' } });
    c.append(t(key));
    return c;
  }));
  // the page list is read when one of these is chosen
  $('dr-pageops').replaceChildren(...[['extract', 'dr.pdf_keep'], ['delete', 'dr.pdf_delete']].map(([op, key]) => {
    const c = choiceBtn('chip dr-choice', { kind: 'pdf_pages', op });
    c.append(t(key));
    return c;
  }));
}

function paintHead() {
  const n = opts ? opts.count : files.length;
  const kinds = opts ? opts.kinds : [];
  const single = files.length === 1;
  $('dr-title').textContent = !files.length ? t('dr.waiting')
    : single ? fileName(files[0])
    : t({ image: 'dr.title_image', audio: 'dr.title_audio', video: 'dr.title_video', document: 'dr.title_document' }[kinds.length === 1 ? kinds[0] : ''] || 'dr.title', { n });
  const names = $('dr-names');
  if (single) {
    names.textContent = String(files[0]).replace(/[\\/][^\\/]*$/, '');
  } else if (files.length) {
    names.textContent = t('dr.names', { first: fileName(files[0]), n: files.length - 1 });
  } else names.textContent = '';
  names.title = files.slice(0, 30).map(fileName).join('\n') + (files.length > 30 ? '\n…' : '');
  const sk = opts ? opts.skipped : 0;
  $('dr-skip').hidden = !sk;
  const skipNames = sk && opts.skipped_names ? opts.skipped_names.join(', ') + (sk > opts.skipped_names.length ? '…' : '') : '';
  $('dr-skip-text').textContent = sk ? `${t('dr.skipped', { n: sk })} ${skipNames}`.trim() : '';
}

function render() {
  paintHead();
  if (!opts) return;
  buildTargets();
  buildPresets();
  buildMerge();
  buildPdf();
  buildCustomCard();
  if (customOpen && !customAvailable()) showCustom(false);
  if (customOpen) (buildCustom(), syncCustom());
  const none = !opts.count;
  $('dr-empty').hidden = !none;
  $('dr-empty').textContent = none ? t('dr.none') : '';
  // one tab stop for the whole set; arrows move between choices
  const all = choices();
  all.forEach((b) => (b.tabIndex = -1));
  const first = all.find((b) => !b.hasAttribute('aria-disabled')) || all[0];
  if (first) first.tabIndex = 0;
  return first;
}

// only what is on screen: a hidden card must not take the focus or the single tab stop
const choices = () => [...document.querySelectorAll('.dr-choice')].filter((b) => b.offsetParent !== null);

function status(text, isErr) {
  const s = $('dr-status');
  s.textContent = text || '';
  s.classList.toggle('err', !!isErr);
  s.setAttribute('role', isErr ? 'alert' : 'status');
}

/* ---------- loading a drop ---------- */
function intro() {
  document.body.classList.remove('enter');
  void document.body.offsetWidth;
  document.body.classList.add('enter');
}

async function load(payload) {
  const my = ++seq;
  files = payload.files || [];
  busy = false;
  document.body.classList.remove('busy');
  opts = null;
  cs = null;
  if (customOpen) showCustom(false);
  status('');
  paintStack(payload.kinds || [], files.length);
  render();
  if (!files.length) return;
  try {
    const [o, s] = await Promise.all([invoke('get_drop_options', { files }), invoke('get_settings').catch(() => null)]);
    if (my !== seq) return;
    opts = o;
    if (s) (ffReady = !!s.ffmpeg_ready), (st = s);
  } catch (e) {
    if (my !== seq) return;
    return status(t('dr.err_options', { err: e }), true);
  }
  paintStack(opts.kinds.length ? opts.kinds : payload.kinds || [], opts.count || files.length);
  const first = render();
  if (payload.custom && customAvailable()) showCustom(true);
  intro();
  $('dr-body').scrollTop = 0;
  if (first && !customOpen) first.focus({ preventScroll: true });
}

/* ---------- choosing ---------- */
async function choose(b) {
  if (busy || !b) return;
  if (b.hasAttribute('aria-disabled')) {
    b.classList.remove('nudge');
    void b.offsetWidth;
    b.classList.add('nudge');
    return status(b.title);
  }
  let choice = JSON.parse(b.dataset.choice);
  if (choice.kind === 'custom_open') return showCustom(true);
  if (choice.kind === 'pdf_pages') {
    const pages = $('dr-pages').value.trim();
    // the same shape the backend accepts: 1-3, 5, 8- (the page count is checked when it runs)
    const ok = pages.split(/[,;]/).map((x) => x.trim()).filter(Boolean)
      .every((x) => /^\d+$/.test(x) || (/^\d*\s*-\s*\d*$/.test(x) && /\d/.test(x)));
    if (!pages || !ok) return $('dr-pages').focus(), status(t('dr.pdf_pages_needed'), true);
    choice = { kind: 'pdf', pdf: { tool: choice.op, pages } };
  }
  go(choice, b);
}

async function go(choice, b) {
  busy = true;
  document.body.classList.add('busy');
  b.classList.add('chosen');
  if (customOpen) $('dr-run').disabled = true;
  status(t('dr.starting'));
  try {
    // the backend queues the jobs and hides this window; the panel takes over from here
    await invoke('run_drop', { files, choice });
  } catch (e) {
    busy = false;
    document.body.classList.remove('busy');
    b.classList.remove('chosen');
    if (customOpen) syncCustom();
    status(String(e), true);
    b.focus();
  }
}

function close() {
  if (busy) return;
  invoke('close_drop_window').catch(() => {});
}

// arrows move to the nearest choice in that direction; Home/End jump to the ends
function arrows(e) {
  const all = choices();
  const cur = document.activeElement;
  const i = all.indexOf(cur);
  if (i < 0) return;
  let next = null;
  if (e.key === 'Home') next = all[0];
  else if (e.key === 'End') next = all[all.length - 1];
  else if (e.key === 'ArrowRight') next = all[i + 1];
  else if (e.key === 'ArrowLeft') next = all[i - 1];
  else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    const down = e.key === 'ArrowDown';
    const r = cur.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    let best = Infinity;
    for (const b of all) {
      const q = b.getBoundingClientRect();
      const dy = down ? q.top - r.bottom : r.top - q.bottom;
      if (dy < -2) continue; // same row or the wrong way
      const score = dy * 4 + Math.abs(q.left + q.width / 2 - cx);
      if (b !== cur && score < best) (best = score), (next = b);
    }
  } else return;
  e.preventDefault();
  if (!next) return;
  cur.tabIndex = -1;
  next.tabIndex = 0;
  next.focus();
  next.scrollIntoView({ block: 'nearest' });
  if (next.hasAttribute('aria-disabled')) status(next.title);
  else if ($('dr-status').classList.contains('err') === false) status('');
}

function relabel() {
  if (!files.length && !opts) return paintHead();
  const focusedChoice = document.activeElement && document.activeElement.dataset.choice;
  render();
  const again = focusedChoice && choices().find((b) => b.dataset.choice === focusedChoice);
  if (again) choices().forEach((b) => (b.tabIndex = b === again ? 0 : -1)), again.focus({ preventScroll: true });
}

/* ---------- more files dragged onto the chooser ---------- */
function dragDrop() {
  const w = T && T.webview && T.webview.getCurrentWebview && T.webview.getCurrentWebview();
  if (!w) return;
  const dz = $('dz');
  let ok = false;
  w.onDragDropEvent(({ payload: p }) => {
    if (p.type === 'enter') ok = !!(p.paths && p.paths.length) && !busy;
    if (p.type === 'enter' || p.type === 'over') return dz.classList.toggle('on', ok);
    dz.classList.remove('on');
    if (p.type === 'drop' && ok) invoke('drop_files', { paths: p.paths }).catch((e) => status(String(e), true));
  });
}

// Nothing is focus-ringed until the keyboard is used; Enter or Space as the very first key only
// shows where the focus is, so a stray key never starts a conversion.
let eatSpace = false;
function keyboardIn(e) {
  if (document.body.classList.contains('kb') || e.key === 'Escape') return false;
  document.body.classList.add('kb');
  eatSpace = e.key === ' ';
  return e.key === 'Enter' || e.key === ' ';
}
document.addEventListener('keyup', (e) => {
  if (e.key === ' ' && eatSpace) (eatSpace = false), e.preventDefault();
});
document.addEventListener('keydown', (e) => {
  if (keyboardIn(e)) return e.preventDefault();
  if (e.key === 'Escape') return e.preventDefault(), customOpen && !busy ? showCustom(false) : close();
  if (e.target.closest && e.target.closest('.dr-choice')) arrows(e);
});
document.addEventListener('click', (e) => {
  const b = e.target.closest('.dr-choice');
  if (b) choose(b);
});
$('dr-cancel').onclick = close;
$('dr-pages').onkeydown = (e) => {
  if (e.key === 'Enter') e.preventDefault(), choose($('dr-pageops').firstElementChild);
};
// The window is created at startup and can load before the backend answers commands, and a
// `drop-files` sent while the page was still loading is lost: ask for the files, retrying briefly.
async function fetchFiles(tries) {
  try {
    const [p, s] = await Promise.all([invoke('get_drop_files'), invoke('get_settings')]);
    if (s.language !== LANG) setLang(s.language), relabel();
    ffReady = !!s.ffmpeg_ready;
    if (!opts || JSON.stringify(p.files) !== JSON.stringify(files)) load(p);
  } catch (e) {
    if (tries > 0) return setTimeout(() => fetchFiles(tries - 1), 250);
    status(t('dr.err_options', { err: e }), true);
  }
}

document.addEventListener('visibilitychange', () => document.visibilityState === 'visible' && fetchFiles(4));
document.addEventListener('DOMContentLoaded', () => {
  initWindowChrome(() => invoke('close_drop_window').catch(() => {}));
  if (T && T.event) {
    T.event.listen('drop-files', (e) => load(e.payload));
    T.event.listen('language-changed', (e) => (setLang(e.payload), relabel()));
  }
  dragDrop();
  fetchFiles(20);
});
