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

  const S = 240, TILE = 64, GAP = 12, OFF = (S - (3 * TILE + 2 * GAP)) / 2;
  const dpr = Math.min(devicePixelRatio || 1, 2);
  cv.width = cv.height = S * dpr;
  ctx.scale(dpr, dpr);

  // logo columns: pale ink, teal, mint; radius grows square -> circle
  const COLS = [["#f2f7f6", "#b7c7c4"], ["#19b59a", "#0e8a75"], ["#9af5df", "#3ccbaa"]];
  const R0 = 0.14, RT = [0.14, 0.375, 0.5];
  const lerp = (a, b, t) => a + (b - a) * t;
  const clamp = (v) => Math.max(0, Math.min(1, v));
  const easeOut = (t) => 1 - Math.pow(1 - t, 3);

  let mode = "install", tp = 0, dp = 0, last = performance.now(), state = "welcome", doneAt = 0;
  const popAt = new Array(9).fill(0);

  function tile(i, t, now) {
    const col = Math.floor(i / 3), row = i % 3;
    const raw = clamp(dp * 9 - i);
    const f = mode === "uninstall" ? 1 - raw : raw; // removing drains the mark
    let x = OFF + col * (TILE + GAP), y = OFF + row * (TILE + GAP);
    const cx = x + TILE / 2, cy = y + TILE / 2;

    // pop when a tile completes
    if (mode === "install" && f >= 0.999 && !popAt[i]) popAt[i] = now;
    let sc = 1;
    if (popAt[i] && !reduce) { const d = (now - popAt[i]) / 1000; sc = 1 + 0.12 * Math.exp(-6 * d) * Math.sin(16 * d + 1.2); }

    // idle breathing while waiting
    let idle = 0;
    if (state === "welcome" && mode === "install" && !reduce) idle = 0.5 + 0.5 * Math.sin(t * 1.4 - (col + row) * 0.7);

    const r = TILE * lerp(R0, RT[col], easeOut(f));
    ctx.save();
    ctx.translate(cx, cy); ctx.scale(sc, sc); ctx.translate(-cx, -cy);
    ctx.beginPath(); ctx.roundRect(x, y, TILE, TILE, r);
    ctx.fillStyle = `rgba(255,255,255,${0.05 + idle * 0.035})`; ctx.fill();
    ctx.lineWidth = 1; ctx.strokeStyle = `rgba(255,255,255,${0.09 + idle * 0.07})`; ctx.stroke();

    if (f > 0.002) {
      ctx.save();
      ctx.clip();
      const level = y + TILE * (1 - f);
      const amp = f >= 1 ? 0 : TILE * 0.05 * Math.sin(Math.PI * f) + 0.6;
      const g = ctx.createLinearGradient(x, y, x + TILE, y + TILE);
      g.addColorStop(0, COLS[col][0]); g.addColorStop(1, COLS[col][1]);
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.moveTo(x - 2, y + TILE + 2);
      for (let px = -2; px <= TILE + 2; px += 4) {
        const w = reduce ? 0 : Math.sin((px / TILE) * 6.283 * 1.3 + t * 3.2 + i * 1.7) * amp;
        ctx.lineTo(x + px, level + w);
      }
      ctx.lineTo(x + TILE + 2, y + TILE + 2); ctx.closePath(); ctx.fill();
      // soft highlight like the logo
      const h = ctx.createRadialGradient(x + TILE * 0.28, y + TILE * 0.18, 0, x + TILE * 0.28, y + TILE * 0.18, TILE * 0.85);
      h.addColorStop(0, "rgba(255,255,255,.34)"); h.addColorStop(0.55, "rgba(255,255,255,0)");
      ctx.fillStyle = h; ctx.fillRect(x, y, TILE, TILE);
      // one sweep of light over the finished mark
      if (doneAt && !reduce) {
        const sp = (now - doneAt) / 1100; // 0..1
        if (sp > 0 && sp < 1) {
          const d = lerp(-0.2, 1.2, sp) * S; // along the diagonal
          const sg = ctx.createLinearGradient(d - 70, d - 70, d + 70, d + 70);
          // gradient is in canvas space, so offset by tile origin through translate
          sg.addColorStop(0, "rgba(255,255,255,0)"); sg.addColorStop(0.5, "rgba(255,255,255,.55)"); sg.addColorStop(1, "rgba(255,255,255,0)");
          ctx.fillStyle = sg; ctx.fillRect(x, y, TILE, TILE);
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
    for (let i = 0; i < 9; i++) tile(i, t, now);

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
      install_to: "Install to", no_admin: "no admin rights needed", browse: "Browse",
      keep: "Keep settings inside the app folder",
      keep_hint: "Settings, history and menu icons go to <b>data</b> next to the app instead of %APPDATA%.",
      install: "Install", update: "Update", uninstall: "Uninstall", cancel: "Cancel", open: "Open Tessera", close: "Close",
      remove_title: "Remove Tessera?", remove_lead: "It leaves the Explorer menu and the app folder is deleted.",
      wipe: "Also delete my settings and history",
      wipe_hint: "Presets, history and the downloaded ffmpeg. Converted files are never touched.",
      done_title: "Tessera is ready", done_lead: "Right-click a file and look for Tessera.",
      done_free: "Free and open source. No account, no ads, no telemetry.", installed_to: "Installed to",
      gone_title: "Tessera is removed", gone_lead: "Your converted files were not touched.",
      gone_fine: "Thanks for trying a free converter.",
      ff_note: "Video support will download the next time Tessera opens.",
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
      install_to: "Kurulum yeri", no_admin: "yönetici izni gerekmez", browse: "Gözat",
      keep: "Ayarları uygulama klasöründe tut",
      keep_hint: "Ayarlar, geçmiş ve menü simgeleri %APPDATA% yerine uygulamanın yanındaki <b>data</b> klasörüne gider.",
      install: "Kur", update: "Güncelle", uninstall: "Kaldır", cancel: "Vazgeç", open: "Tessera'yı aç", close: "Kapat",
      remove_title: "Tessera kaldırılsın mı?", remove_lead: "Explorer menüsünden çıkar ve uygulama klasörü silinir.",
      wipe: "Ayarlarımı ve geçmişimi de sil",
      wipe_hint: "Hazır ayarlar, geçmiş ve indirilen ffmpeg. Dönüştürdüğün dosyalara dokunulmaz.",
      done_title: "Tessera hazır", done_lead: "Bir dosyaya sağ tıkla ve Tessera'yı bul.",
      done_free: "Ücretsiz ve açık kaynak. Hesap, reklam ve izleme yok.", installed_to: "Kurulum yeri:",
      gone_title: "Tessera kaldırıldı", gone_lead: "Dönüştürdüğün dosyalara dokunulmadı.",
      gone_fine: "Ücretsiz bir dönüştürücüyü denediğin için teşekkürler.",
      ff_note: "Video desteği Tessera'nın bir sonraki açılışında indirilecek.",
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
    T.core.invoke("install_start", { req: { dir, portable: document.getElementById("portable").checked } })
      .then((real) => { dirEl.value = real; doneDirPath = real; document.getElementById("doneDir").textContent = t("installed_to") + " " + real; api.start(); })
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
    T.event.listen("install-done", (e) => { document.getElementById("ffNote").textContent = e.payload === false ? t("ff_note") : ""; });
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
  if (q.get("state") === "done") { setState("done"); tp = dp = 1; Object.assign(popAt, new Array(9).fill(1)); }
  if (!T && q.get("mode") === "uninstall") { api.setMode("uninstall"); setGo("uninstall"); }
  if (!T) { applyLang(q.get("lang") || ((navigator.language || "").startsWith("tr") ? "tr" : "en")); ready(); }
  if (q.get("auto")) setTimeout(demo, 800);

  requestAnimationFrame(frame);
})();
