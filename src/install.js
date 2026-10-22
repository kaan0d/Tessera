// Installer screen. One driver, the displayed progress `dp`, moves the mark,
// the percentage, the bar and the glow. Real installs call window.tesseraInstall.set(p, label, step).
(() => {
  const root = document.documentElement;
  const cv = document.getElementById("mark");
  const ctx = cv.getContext("2d");
  const bar = document.getElementById("bar");
  const pct = document.getElementById("pct");
  const strips = [...pct.querySelectorAll(".strip")];
  const labelEl = document.getElementById("label");
  const stepLine = document.getElementById("stepline");
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

  const S = 240, K = S / 256;
  const dpr = Math.min(devicePixelRatio || 1, 2);
  cv.width = cv.height = S * dpr;
  ctx.scale(dpr, dpr);

  const lerp = (a, b, t) => a + (b - a) * t;
  const clamp = (v) => Math.max(0, Math.min(1, v));

  // the arch: 11 pieces in building order (piers, imposts, stones from the feet up, keystone last)
  const ARCH = ["M33 170.25H69A5 5 0 0 1 74 175.25V221.0A5 5 0 0 1 69 226.0H33A5 5 0 0 1 28 221.0V175.25A5 5 0 0 1 33 170.25Z", "M187 170.25H223A5 5 0 0 1 228 175.25V221.0A5 5 0 0 1 223 226.0H187A5 5 0 0 1 182 221.0V175.25A5 5 0 0 1 187 170.25Z", "M26 148.75H76A4 4 0 0 1 80 152.75V160.75A4 4 0 0 1 76 164.75H26A4 4 0 0 1 22 160.75V152.75A4 4 0 0 1 26 148.75Z", "M180 148.75H230A4 4 0 0 1 234 152.75V160.75A4 4 0 0 1 230 164.75H180A4 4 0 0 1 176 160.75V152.75A4 4 0 0 1 180 148.75Z", "M28.18 140.05A100.00 100.00 0 0 1 35.02 109.21A3.2 3.2 0 0 1 39.27 107.66L75.28 124.45A3.2 3.2 0 0 1 76.83 128.70A54.00 54.00 0 0 0 74.31 140.05A3.2 3.2 0 0 1 71.11 143.25L31.38 143.25A3.2 3.2 0 0 1 28.18 140.05Z", "M220.98 109.21A100.00 100.00 0 0 1 227.82 140.05A3.2 3.2 0 0 1 224.62 143.25L184.89 143.25A3.2 3.2 0 0 1 181.69 140.05A54.00 54.00 0 0 0 179.17 128.70A3.2 3.2 0 0 1 180.72 124.45L216.73 107.66A3.2 3.2 0 0 1 220.98 109.21Z", "M40.05 98.42A100.00 100.00 0 0 1 59.28 73.36A3.2 3.2 0 0 1 63.79 73.75L89.33 104.19A3.2 3.2 0 0 1 88.93 108.70A54.00 54.00 0 0 0 81.86 117.92A3.2 3.2 0 0 1 77.60 119.47L41.60 102.68A3.2 3.2 0 0 1 40.05 98.42Z", "M196.72 73.36A100.00 100.00 0 0 1 215.95 98.42A3.2 3.2 0 0 1 214.40 102.68L178.40 119.47A3.2 3.2 0 0 1 174.14 117.92A54.00 54.00 0 0 0 167.07 108.70A3.2 3.2 0 0 1 166.67 104.19L192.21 73.75A3.2 3.2 0 0 1 196.72 73.36Z", "M68.40 65.71A100.00 100.00 0 0 1 96.42 51.12A3.2 3.2 0 0 1 100.34 53.39L110.62 91.76A3.2 3.2 0 0 1 108.36 95.68A54.00 54.00 0 0 0 98.05 101.05A3.2 3.2 0 0 1 93.54 100.65L68.00 70.22A3.2 3.2 0 0 1 68.40 65.71Z", "M159.58 51.12A100.00 100.00 0 0 1 187.60 65.71A3.2 3.2 0 0 1 188.00 70.22L162.46 100.65A3.2 3.2 0 0 1 157.95 101.05A54.00 54.00 0 0 0 147.64 95.68A3.2 3.2 0 0 1 145.38 91.76L155.66 53.39A3.2 3.2 0 0 1 159.58 51.12Z", "M105.06 37.40A111.00 111.00 0 0 1 150.94 37.40A3.2 3.2 0 0 1 153.20 41.32L138.24 97.14A3.2 3.2 0 0 1 134.33 99.40A47.00 47.00 0 0 0 121.67 99.40A3.2 3.2 0 0 1 117.76 97.14L102.80 41.32A3.2 3.2 0 0 1 105.06 37.40Z"];
  const N = ARCH.length;
  const paths = ARCH.map((d) => new Path2D(d));
  const boxes = (() => { // bounding boxes through a hidden svg
    const ns = "http://www.w3.org/2000/svg", svg = document.createElementNS(ns, "svg");
    svg.setAttribute("width", "0"); svg.setAttribute("height", "0"); svg.style.position = "absolute";
    document.body.appendChild(svg);
    const out = ARCH.map((d) => { const p = document.createElementNS(ns, "path"); p.setAttribute("d", d); svg.appendChild(p); const b = p.getBBox(); return { x: b.x, y: b.y, w: b.width, h: b.height }; });
    svg.remove();
    return out;
  })();
  const PAPER = ["#f4f4f2", "#cfcfcb"], CORAL = ["#ff8a6a", "#ff6b4a"];

  let mode = "install", tp = 0, dp = 0, last = performance.now(), state = "welcome", doneAt = 0;
  const popAt = new Array(N).fill(0);

  function piece(i, t, now) {
    const raw = clamp(dp * N - i);
    const f = mode === "uninstall" ? 1 - raw : raw; // removing drains the mark
    const bx = boxes[i], key = i === N - 1;
    const cx = bx.x + bx.w / 2, cy = bx.y + bx.h / 2;

    // a little pop when a stone is set
    if (mode === "install" && f >= 0.999 && !popAt[i]) popAt[i] = now;
    let sc = 1;
    if (popAt[i] && !reduce) { const d = (now - popAt[i]) / 1000; sc = 1 + 0.06 * Math.exp(-6 * d) * Math.sin(16 * d + 1.2); }

    // idle breathing while waiting
    let idle = 0;
    if (state === "welcome" && mode === "install" && !reduce) idle = 0.5 + 0.5 * Math.sin(t * 1.4 - i * 0.55);

    ctx.save();
    ctx.scale(K, K);
    ctx.translate(cx, cy); ctx.scale(sc, sc); ctx.translate(-cx, -cy);
    ctx.fillStyle = key ? `rgba(255,107,74,${0.10 + idle * 0.06})` : `rgba(255,255,255,${0.05 + idle * 0.035})`;
    ctx.fill(paths[i]);
    ctx.lineWidth = 1 / K; ctx.strokeStyle = key ? `rgba(255,107,74,${0.30 + idle * 0.15})` : `rgba(255,255,255,${0.09 + idle * 0.07})`;
    ctx.stroke(paths[i]);

    if (f > 0.002) {
      ctx.save();
      ctx.clip(paths[i]);
      const level = bx.y + bx.h * (1 - f);
      const amp = f >= 1 ? 0 : Math.min(bx.h, 40) * 0.07 * Math.sin(Math.PI * f) + 0.5;
      const grad = ctx.createLinearGradient(bx.x, bx.y, bx.x + bx.w, bx.y + bx.h);
      const col = key ? CORAL : PAPER;
      grad.addColorStop(0, col[0]); grad.addColorStop(1, col[1]);
      ctx.fillStyle = grad;
      ctx.beginPath(); ctx.moveTo(bx.x - 3, bx.y + bx.h + 3);
      for (let px = -3; px <= bx.w + 3; px += 3) {
        const w = reduce ? 0 : Math.sin((px / Math.max(bx.w, 20)) * 6.283 * 1.3 + t * 3.2 + i * 1.7) * amp;
        ctx.lineTo(bx.x + px, level + w);
      }
      ctx.lineTo(bx.x + bx.w + 3, bx.y + bx.h + 3); ctx.closePath(); ctx.fill();
      // soft highlight
      const h = ctx.createRadialGradient(bx.x + bx.w * 0.28, bx.y + bx.h * 0.18, 0, bx.x + bx.w * 0.28, bx.y + bx.h * 0.18, Math.max(bx.w, bx.h) * 0.9);
      h.addColorStop(0, "rgba(255,255,255,.32)"); h.addColorStop(0.55, "rgba(255,255,255,0)");
      ctx.fillStyle = h; ctx.fillRect(bx.x - 2, bx.y - 2, bx.w + 4, bx.h + 4);
      // one sweep of light over the finished arch
      if (doneAt && !reduce) {
        const sp = (now - doneAt) / 1100;
        if (sp > 0 && sp < 1) {
          const d = lerp(-0.2, 1.2, sp) * 256;
          const sg = ctx.createLinearGradient(d - 70, d - 70, d + 70, d + 70);
          sg.addColorStop(0, "rgba(255,255,255,0)"); sg.addColorStop(0.5, "rgba(255,255,255,.55)"); sg.addColorStop(1, "rgba(255,255,255,0)");
          ctx.fillStyle = sg; ctx.fillRect(bx.x - 2, bx.y - 2, bx.w + 4, bx.h + 4);
        }
      }
      ctx.restore();
    }
    ctx.restore();
  }

  function setPct(n) {
    n = Math.max(0, Math.min(100, n));
    const o = n % 10, t = Math.floor(n / 10) % 10, h = n >= 100 ? 1 : 0;
    const v = [h, t, o];
    strips.forEach((s, k) => { s.style.translate = `0 ${-v[k]}em`; });
    pct.classList.toggle("hundred", h === 1); pct.classList.toggle("tens", n >= 10);
  }

  let shownN = -1;
  function frame(now) {
    const dt = Math.min(0.05, (now - last) / 1000); last = now;
    const t = now / 1000;
    dp += (tp - dp) * (1 - Math.exp(-dt * (reduce ? 60 : 5)));
    if (Math.abs(tp - dp) < 0.0015) dp = tp;

    root.style.setProperty("--p", dp.toFixed(4));
    const n = Math.round(dp * 100);
    if (n !== shownN) { shownN = n; setPct(n); bar.setAttribute("aria-valuenow", n); }

    ctx.clearRect(0, 0, S, S);
    for (let i = 0; i < N; i++) piece(i, t, now);

    if (state === "run" && dp >= 1 && tp >= 1) finish(now);
    requestAnimationFrame(frame);
  }

  function setState(s) { state = s; root.dataset.state = s; }
  function finish(now) {
    setState("done"); doneAt = mode === "install" ? now + 250 : 0;
    setTimeout(() => document.getElementById(mode === "install" ? "open" : "close").focus(), 1100);
  }

  let curLabel = "";
  // ---- language: English, or Turkish when Windows is Turkish (the backend says which)
  const DICT = {
    en: {
      install_title: "Install Tessera", update_title: "Update Tessera", lead: "Right-click any file, pick a format, done.",
      install_to: "Install to", browse: "Browse",
      keep: "Keep settings inside the app folder",
      keep_hint: "Settings, history and menu icons go to <b>data</b> next to the app instead of %APPDATA%.",
      install: "Install", update: "Update", uninstall: "Uninstall", cancel: "Cancel", open: "Open Tessera", close: "Close",
      remove_title: "Remove Tessera?", remove_lead: "It leaves the Explorer menu and the app folder is deleted.",
      wipe: "Also delete my settings and history",
      wipe_hint: "Presets, history and the downloaded ffmpeg. Left unticked, they stay for a later reinstall. Converted files are never touched.",
      done_title: "Tessera is ready", done_lead: "Right-click a file and look for Tessera.",
      done_free: "Free and open source. No account, no ads, no telemetry.", installed_to: "Installed to",
      gone_title: "Tessera is removed", gone_lead: "Your converted files were not touched.",
      gone_fine: "Thanks for trying a free converter.",
      left_note: "Could not remove: {items}. You can delete them by hand.",
      left_menu: "the Explorer menu", left_autostart: "the autostart entry", left_shortcut: "the Start menu shortcut",
      left_entry: "the uninstall entry", left_data: "settings and history", left_files: "the program files",
      ff_note: "Video support will download the next time Tessera opens.",
      ff_skipped: "Video support is off. Turn it on any time in Advanced.",
      path: "Add to PATH for terminals",
      path_hint: "Lets you type tessera‑cli in Command Prompt or PowerShell. Most people do not need it.",
      path_note: "PATH updated. Open a new terminal window to use tessera‑cli.",
      left_path: "the PATH entry",
      ffdl: "Download video support now (about 170 MB)",
      ffdl_hint: "Needed for video, MP3, OGG and AAC. You can also do it later.",
      "Skipping video support": "Skipping video support",
      step: "{n} of {t}",
      "Copying files": "Copying files", "Registering the menu": "Registering the menu",
      "Adding icons and shortcuts": "Adding icons and shortcuts", "Downloading video support": "Downloading video support",
      "Finishing up": "Finishing up", "Closing Tessera": "Closing Tessera", "Removing the menu": "Removing the menu",
      "Removing shortcuts and entries": "Removing shortcuts and entries", "Deleting files": "Deleting files",
      tips: [
        "Free and open source. MIT licensed, nothing to unlock.",
        "Paid converters do the same job. You do not need one.",
        "No account, no ads, no telemetry.",
        "Your files never leave this computer.",
        "A modern interface in one small app.",
        "Right-click any file, pick a format, done.",
      ],
    },
    tr: {
      install_title: "Tessera'yı kur", update_title: "Tessera'yı güncelle", lead: "Herhangi bir dosyaya sağ tıkla, biçimi seç, bitti.",
      install_to: "Kurulum yeri", browse: "Gözat",
      keep: "Ayarları uygulama klasöründe tut",
      keep_hint: "Ayarlar, geçmiş ve menü simgeleri %APPDATA% yerine uygulamanın yanındaki <b>data</b> klasörüne gider.",
      install: "Kur", update: "Güncelle", uninstall: "Kaldır", cancel: "Vazgeç", open: "Tessera'yı aç", close: "Kapat",
      remove_title: "Tessera kaldırılsın mı?", remove_lead: "Explorer menüsünden çıkar ve uygulama klasörü silinir.",
      wipe: "Ayarlarımı ve geçmişimi de sil",
      wipe_hint: "Hazır ayarlar, geçmiş ve indirilen ffmpeg. İşaretlemezsen sonraki kurulum için saklanır. Dönüştürdüğün dosyalara dokunulmaz.",
      done_title: "Tessera hazır", done_lead: "Bir dosyaya sağ tıkla ve Tessera'yı bul.",
      done_free: "Ücretsiz ve açık kaynak. Hesap, reklam ve izleme yok.", installed_to: "Kurulum yeri:",
      gone_title: "Tessera kaldırıldı", gone_lead: "Dönüştürdüğün dosyalara dokunulmadı.",
      gone_fine: "Ücretsiz bir dönüştürücüyü denediğin için teşekkürler.",
      left_note: "Kaldırılamayanlar: {items}. Elle silebilirsin.",
      left_menu: "Explorer menüsü", left_autostart: "otomatik başlatma kaydı", left_shortcut: "Başlat menüsü kısayolu",
      left_entry: "kaldırma kaydı", left_data: "ayarlar ve geçmiş", left_files: "program dosyaları",
      ff_note: "Video desteği Tessera'nın bir sonraki açılışında indirilecek.",
      ff_skipped: "Video desteği kapalı. İstediğin zaman Gelişmiş bölümünden açabilirsin.",
      path: "Terminaller için PATH'e ekle",
      path_hint: "Komut İstemi ve PowerShell'de tessera‑cli yazabilirsin. Çoğu kişiye gerekmez.",
      path_note: "PATH güncellendi. tessera‑cli için yeni bir terminal penceresi aç.",
      left_path: "PATH kaydı",
      ffdl: "Video desteğini şimdi indir (yaklaşık 170 MB)",
      ffdl_hint: "Video, MP3, OGG ve AAC için gerekli. Sonra da yapabilirsin.",
      "Skipping video support": "Video desteği atlanıyor",
      step: "Adım {n} / {t}",
      "Copying files": "Dosyalar kopyalanıyor", "Registering the menu": "Menü kaydediliyor",
      "Adding icons and shortcuts": "Simgeler ve kısayollar ekleniyor", "Downloading video support": "Video desteği indiriliyor",
      "Finishing up": "Bitiriliyor", "Closing Tessera": "Tessera kapatılıyor", "Removing the menu": "Menü kaldırılıyor",
      "Removing shortcuts and entries": "Kısayollar ve kayıtlar kaldırılıyor", "Deleting files": "Dosyalar siliniyor",
      tips: [
        "Ücretsiz ve açık kaynak. MIT lisanslı, açılacak kilit yok.",
        "Ücretli dönüştürücüler aynı işi yapıyor. Birine ihtiyacın yok.",
        "Hesap yok, reklam yok, izleme yok.",
        "Dosyaların bu bilgisayardan hiç çıkmaz.",
        "Tek küçük uygulamada modern bir arayüz.",
        "Herhangi bir dosyaya sağ tıkla, biçimi seç, bitti.",
      ],
    },
  };
  let lang = "en";
  const t = (k) => DICT[lang][k] ?? DICT.en[k] ?? k;
  function applyLang(l) {
    lang = DICT[l] ? l : "en";
    document.documentElement.lang = lang;
    document.querySelectorAll("[data-t]").forEach((el) => { el.textContent = t(el.dataset.t); });
    document.querySelectorAll("[data-h]").forEach((el) => { el.innerHTML = t(el.dataset.h); }); // our own strings
    document.querySelectorAll("[data-label]").forEach((el) => el.setAttribute("aria-label", t(el.dataset.label)));
    document.getElementById("bar").setAttribute("aria-label", t("install"));
    if (curStep) stepLine.textContent = fmtStep();
    if (curLabel) labelEl.textContent = t(curLabel);
    if (doneDirPath) document.getElementById("doneDir").textContent = t("installed_to") + " " + doneDirPath;
  }
  let curStep = 0, totalSteps = 5, doneDirPath = "";
  const fmtStep = () => t("step").replace("{n}", curStep).replace("{t}", totalSteps);
  function setLabel(text, step) {
    if (step) { curStep = step; stepLine.textContent = fmtStep(); }
    if (!text || text === curLabel) return;
    curLabel = text;
    if (reduce) { labelEl.textContent = t(text); return; }
    labelEl.animate([{ opacity: 1, translate: "0 0" }, { opacity: 0, translate: "0 -8px" }], { duration: 140, easing: "ease-in" }).onfinish = () => {
      labelEl.textContent = t(text);
      labelEl.animate([{ opacity: 0, translate: "0 10px" }, { opacity: 1, translate: "0 0" }], { duration: 380, easing: "cubic-bezier(.16,1,.3,1)" });
    };
  }

  const tipEl = document.getElementById("tip");
  let tipIdx = -1, tipTimer = 0;
  function nextTip() {
    const TIPS = t("tips");
    tipIdx = (tipIdx + 1) % TIPS.length;
    if (reduce || tipIdx === 0) { tipEl.textContent = TIPS[tipIdx]; return; }
    tipEl.classList.add("out");
    setTimeout(() => {
      tipEl.textContent = TIPS[tipIdx];
      tipEl.classList.remove("out"); tipEl.classList.add("pre");
      void tipEl.offsetWidth;
      tipEl.classList.remove("pre");
    }, 500);
  }
  function startTips() { if (mode !== "install" || tipTimer) return; nextTip(); tipTimer = setInterval(nextTip, 2200); }

  const api = {
    start() { if (state !== "welcome") return; setState("run"); startTips(); totalSteps = mode === "install" ? 5 : 4; setLabel(mode === "install" ? "Copying files" : "Closing Tessera", 1); },
    setMode(m) { mode = m; root.dataset.mode = m; if (m === "uninstall") popAt.fill(1); },
    set(p, label, step) { tp = clamp(p); setLabel(label, step); },
  };
  window.tesseraInstall = api;

  // preview simulation: irregular, like real file copying
  const STAGES_UN = [[0.2, "Closing Tessera", 1, 1200], [0.5, "Removing the menu", 2, 1400], [0.75, "Removing shortcuts and entries", 3, 1400], [1, "Deleting files", 4, 1800]];
  const STAGES_IN = [[0.06, "Copying files", 1, 900], [0.09, "Registering the menu", 2, 600], [0.12, "Adding icons and shortcuts", 3, 600], [0.96, "Downloading video support", 4, 6000], [1, "Finishing up", 5, 700]];
  function demo() {
    api.start();
    let from = 0, k = 0;
    const STAGES = mode === "install" ? STAGES_IN : STAGES_UN;
    const next = () => {
      if (k >= STAGES.length) return;
      const [to, label, step, ms] = STAGES[k++];
      api.set(from, label, step);
      const t0 = performance.now(), a = from;
      const tick = (now) => {
        const u = clamp((now - t0) / ms);
        const jitter = Math.sin(u * 17) * 0.015 * (1 - u); // stutters, never goes backwards overall
        api.set(Math.max(a, lerp(a, to, u) + jitter), label, step);
        if (u < 1) requestAnimationFrame(tick); else { from = to; next(); }
      };
      requestAnimationFrame(tick);
    };
    next();
  }

  const T = window.__TAURI__;
  const dirEl = document.getElementById("dir");
  const errEl = document.getElementById("err");
  const SAMPLE = [String.raw`D:\Apps\Tessera`, String.raw`C:\Users\you\AppData\Local\Tessera`]; // preview only

  function showError(msg) {
    errEl.textContent = document.getElementById("err2").textContent = msg;
    setState("welcome"); tp = dp = 0; popAt.fill(mode === "install" ? 0 : 1); curLabel = "";
  }

  document.getElementById("go").addEventListener("click", () => {
    errEl.textContent = document.getElementById("err2").textContent = "";
    if (mode === "uninstall") {
      if (!T) return demo();
      return T.core.invoke("uninstall_start", { keepData: !document.getElementById("wipe").checked }).then(() => api.start()).catch((e) => showError(String(e)));
    }
    const dir = dirEl.value.trim();
    doneDirPath = dir; document.getElementById("doneDir").textContent = t("installed_to") + " " + dir;
    if (!T) return demo();
    T.core.invoke("install_start", { req: { dir, portable: document.getElementById("portable").checked, ffmpeg: document.getElementById("ffdl").checked, path: document.getElementById("pathadd").checked } })
      .then((real) => { document.getElementById("pathNote").textContent = document.getElementById("pathadd").checked ? t("path_note") : ""; dirEl.value = real; doneDirPath = real; document.getElementById("doneDir").textContent = t("installed_to") + " " + real; api.start(); })
      .catch((e) => showError(String(e)));
  });
  document.getElementById("browse").addEventListener("click", () => {
    if (T) T.core.invoke("pick_install_dir").then((p) => { if (p) dirEl.value = p; }).catch(() => {});
    else dirEl.value = SAMPLE[(SAMPLE.indexOf(dirEl.value) + 1) % SAMPLE.length];
  });
  document.getElementById("x").addEventListener("click", () => { if (T) T.core.invoke("install_close"); else location.reload(); });
  document.getElementById("cancel").addEventListener("click", () => { if (T) T.core.invoke("install_close"); else location.reload(); });
  document.getElementById("close").addEventListener("click", () => { if (T) T.core.invoke("install_close"); else location.reload(); });
  document.getElementById("open").addEventListener("click", () => { if (T) T.core.invoke("install_open"); else location.reload(); });

  const ready = () => document.body.classList.add("ready");
  const goEl = document.getElementById("go");
  function setGo(key) { goEl.dataset.t = key; goEl.textContent = t(key); }
  // empty areas drag the frameless window
  document.querySelectorAll(".stage,.col,.views,.view,.pct,.foot,.actions,.mark-cv").forEach((el) => el.setAttribute("data-tauri-drag-region", ""));
  if (T) {
    T.event.listen("install-progress", (e) => api.set(e.payload.p, e.payload.label, e.payload.step));
    T.event.listen("install-error", (e) => showError(String(e.payload)));
    T.event.listen("uninstall-left", (e) => {
      const items = (e.payload || []).map((k) => t("left_" + k)).join(", ");
      document.getElementById("leftNote").textContent = items ? t("left_note").replace("{items}", items) : "";
    });
    T.event.listen("install-done", (e) => { document.getElementById("ffNote").textContent = e.payload === "failed" ? t("ff_note") : e.payload === "skipped" ? t("ff_skipped") : ""; });
    T.core.invoke("install_defaults").then((d) => {
      applyLang(d.lang);
      if (d.mode === "uninstall") {
        api.setMode("uninstall");
        setGo("uninstall");
      } else {
        dirEl.value = d.dir;
        document.getElementById("portable").checked = d.portable;
        if (d.installed) { document.getElementById("welcomeTitle").dataset.t = "update_title"; setGo("update"); applyLang(d.lang); }
      }
      ready();
    }).catch(ready);
    setTimeout(ready, 1500);
  }

  // ?state=run&p=0.6 / ?state=done for screenshots
  const q = new URLSearchParams(location.search);
  if (q.get("state") === "run") { setState("run"); tp = dp = parseFloat(q.get("p") || "0.5"); curLabel = "Registering the menu"; labelEl.textContent = t(curLabel); curStep = 2; stepLine.textContent = fmtStep(); }
  if (q.get("state") === "done") { setState("done"); tp = dp = 1; Object.assign(popAt, new Array(N).fill(1)); }
  if (!T && q.get("mode") === "uninstall") { api.setMode("uninstall"); setGo("uninstall"); }
  if (!T) { applyLang(q.get("lang") || ((navigator.language || "").startsWith("tr") ? "tr" : "en")); ready(); }
  if (q.get("auto")) setTimeout(demo, 800);

  requestAnimationFrame(frame);
})();
