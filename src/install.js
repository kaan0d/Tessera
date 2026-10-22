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
  const stepEl = document.getElementById("step");
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

  let tp = 0, dp = 0, last = performance.now(), state = "welcome", doneAt = 0;
  const popAt = new Array(9).fill(0);

  function tile(i, t, now) {
    const col = Math.floor(i / 3), row = i % 3;
    const f = reduce ? clamp(dp * 9 - i) : clamp(dp * 9 - i);
    let x = OFF + col * (TILE + GAP), y = OFF + row * (TILE + GAP);
    const cx = x + TILE / 2, cy = y + TILE / 2;

    // pop when a tile completes
    if (f >= 0.999 && !popAt[i]) popAt[i] = now;
    let sc = 1;
    if (popAt[i] && !reduce) { const d = (now - popAt[i]) / 1000; sc = 1 + 0.12 * Math.exp(-6 * d) * Math.sin(16 * d + 1.2); }

    // idle breathing while waiting
    let idle = 0;
    if (state === "welcome" && !reduce) idle = 0.5 + 0.5 * Math.sin(t * 1.4 - (col + row) * 0.7);

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
    setState("done"); doneAt = now + 250;
    setTimeout(() => document.getElementById("open").focus(), 1100);
  }

  let curLabel = "";
  function setLabel(text, step) {
    if (step) stepEl.textContent = step;
    if (!text || text === curLabel) return;
    curLabel = text;
    if (reduce) { labelEl.textContent = text; return; }
    labelEl.animate([{ opacity: 1, translate: "0 0" }, { opacity: 0, translate: "0 -8px" }], { duration: 140, easing: "ease-in" }).onfinish = () => {
      labelEl.textContent = text;
      labelEl.animate([{ opacity: 0, translate: "0 10px" }, { opacity: 1, translate: "0 0" }], { duration: 380, easing: "cubic-bezier(.16,1,.3,1)" });
    };
  }

  const api = {
    start() { if (state !== "welcome") return; setState("run"); setLabel("Copying files", 1); },
    set(p, label, step) { tp = clamp(p); setLabel(label, step); },
  };
  window.tesseraInstall = api;

  // preview simulation: irregular, like real file copying
  const STAGES = [[0.55, "Copying files", 1, 3600], [0.76, "Registering the menu", 2, 1500], [0.93, "Adding icons and shortcuts", 3, 1400], [1, "Finishing up", 4, 900]];
  function demo() {
    api.start();
    let from = 0, k = 0;
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
    errEl.textContent = msg;
    setState("welcome"); tp = dp = 0; popAt.fill(0); curLabel = "";
  }

  document.getElementById("go").addEventListener("click", () => {
    errEl.textContent = "";
    const dir = dirEl.value.trim();
    document.getElementById("doneDir").textContent = "Installed to " + dir;
    if (!T) return demo();
    T.core.invoke("install_start", { req: { dir, portable: document.getElementById("portable").checked } })
      .then((real) => { dirEl.value = real; document.getElementById("doneDir").textContent = "Installed to " + real; api.start(); })
      .catch((e) => showError(String(e)));
  });
  document.getElementById("browse").addEventListener("click", () => {
    if (T) T.core.invoke("pick_install_dir").then((p) => { if (p) dirEl.value = p; }).catch(() => {});
    else dirEl.value = SAMPLE[(SAMPLE.indexOf(dirEl.value) + 1) % SAMPLE.length];
  });
  document.getElementById("close").addEventListener("click", () => { if (T) T.core.invoke("install_close"); else location.reload(); });
  document.getElementById("open").addEventListener("click", () => { if (T) T.core.invoke("install_open"); else location.reload(); });

  if (T) {
    T.event.listen("install-progress", (e) => api.set(e.payload.p, e.payload.label, e.payload.step));
    T.event.listen("install-error", (e) => showError(String(e.payload)));
    T.core.invoke("install_defaults").then((d) => {
      dirEl.value = d.dir;
      document.getElementById("portable").checked = d.portable;
      if (d.installed) {
        document.querySelector(".v-welcome h1").textContent = "Update Tessera";
        document.getElementById("go").textContent = "Update";
      }
    }).catch(() => {});
  }

  // ?state=run&p=0.6 / ?state=done for screenshots
  const q = new URLSearchParams(location.search);
  if (q.get("state") === "run") { setState("run"); tp = dp = parseFloat(q.get("p") || "0.5"); curLabel = "Registering the menu"; labelEl.textContent = curLabel; stepEl.textContent = "2"; }
  if (q.get("state") === "done") { setState("done"); tp = dp = 1; Object.assign(popAt, new Array(9).fill(1)); }
  if (q.get("auto")) setTimeout(demo, 800);

  requestAnimationFrame(frame);
})();
