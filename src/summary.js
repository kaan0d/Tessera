'use strict';
/* One-line preset summary ("JPG · max 1600 px · under 1.5 MB"), shared by the settings window and the chooser. */

const num = (n, d = 1) => new Intl.NumberFormat(LANG, { maximumFractionDigits: d }).format(n);

function fmtBytes(b) {
  return b >= 1e6 ? t('unit.mb', { n: num(b / 1e6) }) : t('unit.kb', { n: num(b / 1e3, 0) });
}

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

/** Small "ffmpeg" tag on a format that is converted or opened with ffmpeg. */
function ffTag() {
  const s = document.createElement('span');
  s.className = 'badge tag';
  s.textContent = 'ffmpeg';
  return s;
}
