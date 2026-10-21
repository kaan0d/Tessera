'use strict';
const T = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const invoke = (cmd, args) =>
  T && T.core ? T.core.invoke(cmd, args) : Promise.reject('Uygulama arka ucuna bağlanamadı.');

let s = null; // current settings object
let saveTimer, savedTimer, toastTimer, ffTimer;

$('app-title').textContent = document.title;

function showError(msg) {
  $('error').hidden = !msg;
  $('error').textContent = msg ? String(msg) : '';
}

function setQuality(v) {
  $('quality').value = v;
  $('qval').textContent = v;
  $('quality').style.setProperty('--fill', ((v - 1) / 99) * 100 + '%');
}

function render() {
  $('autostart').checked = s.autostart;
  const fixed = s.output_dir.kind === 'fixed';
  document.querySelector(`input[name=dir][value=${fixed ? 'fixed' : 'same_as_source'}]`).checked = true;
  $('path').hidden = !fixed;
  $('path').textContent = fixed ? s.output_dir.path || '' : '';
  $('conflict').value = s.conflict;
  $('warn').hidden = s.conflict !== 'replace_original';
  setQuality(s.jpeg_quality);
  $('ff-notice').hidden = !!s.ffmpeg_notice_seen;
}

function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await invoke('set_settings', { settings: s });
      showError('');
      $('saved-text').textContent = 'Kaydedildi';
      $('saved').classList.add('on');
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => $('saved').classList.remove('on'), 1600);
    } catch (e) {
      showError('Ayarlar kaydedilemedi: ' + e);
    }
  }, 300);
}

function change(patch) {
  Object.assign(s, patch);
  render();
  save();
}

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
  $('autostart').onchange = (e) => change({ autostart: e.target.checked });
  document.querySelectorAll('input[name=dir]').forEach((r) => {
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
  $('quality').oninput = (e) => {
    s.jpeg_quality = +e.target.value;
    setQuality(s.jpeg_quality);
    save();
  };
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
  if (document.visibilityState !== 'visible') return;
  document.body.classList.remove('enter');
  void document.body.offsetWidth;
  document.body.classList.add('enter');
}
document.addEventListener('visibilitychange', intro);

(async () => {
  bind();
  intro();
  try {
    s = await invoke('get_settings');
    render();
  } catch (e) {
    showError('Ayarlar yüklenemedi: ' + e);
    document.querySelectorAll('main input,main select,main button').forEach((x) => (x.disabled = true));
    return;
  }
  refreshFfmpeg();
})();
