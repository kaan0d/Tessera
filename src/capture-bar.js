'use strict';
/* Control bar shown while recording: elapsed time, Stop (saves) and Cancel (discards). */
const T = window.__TAURI__;
const invoke = (cmd, args) => (T && T.core ? T.core.invoke(cmd, args) : Promise.reject(t('err.backend')));
const time = document.getElementById('time');
let base = 0, at = performance.now();

setLang('en');
const two = (n) => String(n).padStart(2, '0');
function paint() {
  const s = Math.floor(base + (performance.now() - at) / 1000);
  time.textContent = s >= 3600 ? `${Math.floor(s / 3600)}:${two(Math.floor(s / 60) % 60)}:${two(s % 60)}` : `${two(Math.floor(s / 60))}:${two(s % 60)}`;
}
async function sync() {
  try {
    const st = await invoke('capture_status');
    if (st) (base = st.secs), (at = performance.now());
  } catch { /* the bar closes when the recording ends */ }
}
if (T && T.core) {
  T.core.invoke('get_settings').then((s) => setLang(s.language)).catch(() => {});
  sync();
  setInterval(sync, 5000);
}
setInterval(paint, 500);
document.getElementById('stop').onclick = () => invoke('capture_stop').catch(() => {});
document.getElementById('cancel').onclick = () => invoke('capture_cancel').catch(() => {});
