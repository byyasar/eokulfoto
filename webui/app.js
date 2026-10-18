/* ─────────────────────────────────────────────────────────────────────────
   Webcam Capture — arayüz mantığı
   pywebview köprüsünü (window.pywebview.api) kullanır.
   ───────────────────────────────────────────────────────────────────────── */
(() => {
  "use strict";

  const $ = (id) => document.getElementById(id);

  // ÖNEMLİ: pywebview, window.pywebview köprüsünü sayfa tamamen yüklendikten
  // SONRA enjekte eder. Bu yüzden köprüyü bir kez önbelleğe almıyoruz; her
  // çağrıda güncel değeri okuyoruz.
  const getApi = () => (window.pywebview && window.pywebview.api) || null;

  const state = {
    running: false,
    cfg: null,
    zoom: 1.0,
    sliderDragging: false,
    auto: JSON.parse(localStorage.getItem("wc_auto") || "false"),
    interval: parseFloat(localStorage.getItem("wc_interval") || "2"),
  };

  let previewTimer = null;
  let autoTimer = null;
  let autoBusy = false;
  let captureCooldown = 0;
  let zoomDebounceTimer = null;

  /* ─────────────── köprü yardımcıları ─────────────── */

  function call(name, ...args) {
    const bridge = getApi();
    if (!bridge || typeof bridge[name] !== "function") {
      return Promise.reject(new Error("pywebview köprüsü bulunamadı"));
    }
    return bridge[name](...args);
  }

  const hasBridge = () => !!getApi();

  function toast(msg, ms = 2800) {
    const t = $("toast");
    t.textContent = msg;
    t.classList.remove("hidden");
    t.classList.add("show");
    clearTimeout(t._h);
    t._h = setTimeout(() => t.classList.remove("show"), ms);
  }

  function setStatus(text, kind = "") {
    $("statusText").textContent = text;
    const dot = $("statusDot");
    dot.className = "status-dot" + (kind ? " " + kind : "");
  }

  /* ─────────────── yakınlaştırma yardımcıları ─────────────── */

  function updateZoomUI(z) {
    state.zoom = Math.max(0.4, Math.min(2.5, Math.round(z * 100) / 100));
    const slider = $("zoomSlider");
    if (slider && !state.sliderDragging) {
      slider.value = state.zoom;
    }
    const badge = $("zoomBadge");
    if (badge) {
      badge.textContent = state.zoom.toFixed(1) + "x";
    }
    const sZoom = $("sZoom");
    if (sZoom) {
      sZoom.value = state.zoom;
      if ($("sZoomVal")) $("sZoomVal").textContent = state.zoom.toFixed(1) + "x";
      setSliderFill(sZoom);
    }
    if (slider) {
      const min = parseFloat(slider.min) || 0.5;
      const max = parseFloat(slider.max) || 2.5;
      const pct = ((state.zoom - min) / (max - min)) * 100;
      slider.style.setProperty("--zoom-fill", pct + "%");
    }
  }

  async function setZoom(z, updateServer = true) {
    const clamped = Math.max(0.4, Math.min(2.5, Math.round(z * 100) / 100));
    updateZoomUI(clamped);
    if (updateServer) {
      clearTimeout(zoomDebounceTimer);
      zoomDebounceTimer = setTimeout(async () => {
        try {
          await call("set_zoom", state.zoom);
        } catch (e) {
          /* geçici köprü hatası */
        }
      }, 40);
    }
  }

  /* ─────────────── yüz algılama yardımcıları ─────────────── */

  function updateFaceDetectUI(enabled) {
    if (state.cfg) state.cfg.face_detect = !!enabled;
    const btn = $("btnQuickFace");
    const lbl = $("quickFaceLabel");
    if (btn) {
      btn.classList.toggle("active", !!enabled);
    }
    if (lbl) {
      lbl.textContent = enabled ? "Yüz Takibi: Açık" : "Yüz Takibi: Kapalı";
    }
    if ($("sFace")) {
      $("sFace").checked = !!enabled;
    }
    const freqControls = $("faceFreqControls");
    if (freqControls) {
      freqControls.style.opacity = enabled ? "1" : "0.45";
      freqControls.style.pointerEvents = enabled ? "auto" : "none";
    }
  }

  function updateFaceEveryUI(every) {
    const val = Math.max(1, Math.min(60, parseInt(every, 10) || 3));
    if (state.cfg) state.cfg.face_detect_every = val;
    const badge = $("quickEveryBadge");
    if (badge) {
      badge.textContent = val === 1 ? "Her 1 kare (Akıcı)" : `Her ${val} karede 1`;
    }
    if ($("sEvery")) {
      $("sEvery").value = val;
      setSliderFill($("sEvery"));
    }
    if ($("sEveryNum")) {
      $("sEveryNum").value = val;
    }
    if ($("sEveryVal")) {
      let desc = "";
      if (val === 1) desc = "1 kare (Her karede / Ultra Akıcı)";
      else if (val === 2) desc = "2 kare (Hızlı)";
      else if (val === 3) desc = "3 kare (Önerilen)";
      else desc = `${val} kare`;
      $("sEveryVal").textContent = desc;
    }
    document.querySelectorAll(".freq-chip").forEach((chip) => {
      const chipVal = parseInt(chip.getAttribute("data-every"), 10);
      chip.classList.toggle("active", chipVal === val);
    });
  }

  async function toggleFaceDetect(forcedState) {
    const current = state.cfg ? !!state.cfg.face_detect : true;
    const target = forcedState !== undefined ? !!forcedState : !current;
    updateFaceDetectUI(target);
    try {
      const res = await call("set_face_detect", target);
      if (res && res.ok) {
        toast(target ? "Yüz takibi açıldı" : "Yüz takibi kapatıldı", 1800);
      }
    } catch (e) {
      /* köprü hatası */
    }
  }

  async function setFaceEvery(val) {
    const target = Math.max(1, Math.min(60, parseInt(val, 10) || 3));
    updateFaceEveryUI(target);
    try {
      await call("set_face_detect_every", target);
    } catch (e) {
      /* köprü hatası */
    }
  }

  /* ─────────────── durum güncelleme ─────────────── */

  function applyState(raw) {
    if (!raw) return;
    state.cfg = raw;
    $("resLabel").textContent = `${raw.width} × ${raw.height}`;
    $("targetLabel").textContent = `${raw.width} × ${raw.height}`;
    const qInfo = (raw.image_format || "png").toLowerCase() === "jpg" ? ` (%${raw.jpeg_quality || 95})` : "";
    $("targetMeta").textContent =
      `${(raw.image_format || "png").toUpperCase()}${qInfo} · ${raw.output_dir || "captures"}`;
    $("countLabel").textContent = String(raw.count || 0);
    $("lastLabel").textContent = raw.last_name || "—";
    $("dirLabel").textContent = raw.output_dir || "captures";
    if ($("nameExtBadge")) {
      $("nameExtBadge").textContent = `.${(raw.image_format || "png").toLowerCase()}`;
    }
    if ($("tagEdevlet")) {
      $("tagEdevlet").textContent = raw.edevlet_limit !== false ? "e-Devlet uyumlu" : "Özel Kalite";
      $("tagEdevlet").classList.toggle("highlight", raw.edevlet_limit !== false);
    }
    if (raw.frame_zoom !== undefined) {
      updateZoomUI(parseFloat(raw.frame_zoom) || 1.0);
    }
    if (raw.face_detect !== undefined) {
      updateFaceDetectUI(raw.face_detect);
    }
    if (raw.face_detect_every !== undefined) {
      updateFaceEveryUI(raw.face_detect_every);
    }
    if (raw.cameras) {
      populateCameras(raw.cameras, raw.camera_index);
    }
    if (raw.running !== undefined) setCameraUI(raw.running);
    if (raw.status) setStatus(raw.status, raw.running ? "ok" : "");
  }

  function populateCameras(cameras, selectedIndex) {
    const sel = $("sCamera");
    if (!sel) return;
    sel.innerHTML = "";
    if (Array.isArray(cameras) && cameras.length > 0) {
      cameras.forEach((cam) => {
        const opt = document.createElement("option");
        opt.value = cam.index;
        opt.textContent = `${cam.name} (Kamera ${cam.index})`;
        if (cam.index === selectedIndex) {
          opt.selected = true;
        }
        sel.appendChild(opt);
      });
    } else {
      const opt = document.createElement("option");
      opt.value = selectedIndex || 0;
      opt.textContent = `Kamera ${selectedIndex || 0}`;
      opt.selected = true;
      sel.appendChild(opt);
    }
    if (selectedIndex !== undefined) {
      sel.value = selectedIndex;
    }
  }

  function setCameraUI(running) {
    state.running = !!running;
    $("camBadge").textContent = running ? "açık" : "kapalı";
    $("camBadge").className = "badge" + (running ? " on" : "");
    $("btnStart").classList.toggle("on", running);
    $("btnStart").classList.toggle("active-side", running);
    $("btnStop").classList.toggle("on", !running);
    $("btnStop").classList.toggle("active-side", !running);
    const livePill = $("livePill");
    if (livePill) {
      livePill.classList.toggle("hidden", !running);
    }
    if (running) {
      $("liveEmpty").classList.add("hidden");
      $("liveImg").classList.remove("hidden");
    } else {
      $("liveImg").classList.add("hidden");
      $("liveEmpty").classList.remove("hidden");
      $("faceLine").textContent = "Kamera kapalı";
      $("faceStatus").textContent = "yüz modu kapalı";
    }
  }

  function updateFromSnapshot(s) {
    if (!s) return;
    if (s.running !== undefined) setCameraUI(s.running);

    if (s.live) {
      $("liveImg").src = "data:image/jpeg;base64," + s.live;
    }
    if (s.target) {
      const mime = s.target.startsWith("/9j/") ? "image/jpeg" : "image/png";
      $("targetImg").src = `data:${mime};base64,` + s.target;
      $("targetImg").classList.add("visible");
      $("targetEmpty").classList.remove("visible");
    }

    if (s.zoom !== undefined && !state.sliderDragging) {
      if (Math.abs(s.zoom - state.zoom) > 0.04) {
        updateZoomUI(s.zoom);
      }
    }

    // yüz durumu
    const cfg = state.cfg || {};
    const faceDetectActive = s.face_detect !== undefined ? s.face_detect : (cfg.face_detect !== false);
    if (faceDetectActive) {
      const faceCount = s.face_count || (s.face ? 1 : 0);
      const everyTxt = s.face_detect_every || cfg.face_detect_every || 3;
      if (s.face && faceCount > 0) {
        const countTxt = faceCount > 1 ? `${faceCount} yüz algılandı (en büyük ortalanıyor)` : "Yüz algılandı — ortalanıyor";
        $("faceLine").textContent = `● ${countTxt}  (kadraj ${s.box_size}, ${state.zoom.toFixed(1)}x, her ${everyTxt} kareden 1)`;
        $("faceLine").style.color = "var(--green)";
        $("faceStatus").textContent = `${faceCount} yüz aktif`;
      } else {
        $("faceLine").textContent = `○ Yüz aranıyor (her ${everyTxt} kareden 1) — merkezi kadraj aktif (${state.zoom.toFixed(1)}x)`;
        $("faceLine").style.color = "var(--orange)";
        $("faceStatus").textContent = "yüz aranıyor";
      }
    } else {
      $("faceLine").textContent = `Yüz takibi kapalı — merkezi kadraj aktif (${state.zoom.toFixed(1)}x)`;
      $("faceLine").style.color = "var(--text-2)";
      $("faceStatus").textContent = "merkezi kadraj";
    }

    if (s.status) setStatus(s.status, "ok");
    $("countLabel").textContent = String(s.count || 0);
    $("lastLabel").textContent = s.last_name || "—";
  }

  /* ─────────────── önizleme döngüsü ─────────────── */

  let previewActive = false;
  let previewBusy = false;

  function startPreviewLoop() {
    if (previewActive) return;
    previewActive = true;

    async function loop() {
      if (!previewActive) return;
      if (state.running && !previewBusy) {
        previewBusy = true;
        try {
          const s = await call("get_snapshot");
          updateFromSnapshot(s);
        } catch (err) {
          /* geçici köprü hatası */
        } finally {
          previewBusy = false;
        }
      }
      if (previewActive) {
        setTimeout(loop, 25);
      }
    }

    loop();
  }

  function stopPreviewLoop() {
    previewActive = false;
  }

  /* ─────────────── kamera kontrolü ─────────────── */

  async function toggleCamera(start) {
    try {
      const res = await call(start ? "start_camera" : "stop_camera");
      setStatus(res.message || (start ? "Kamera açıldı." : "Kamera durduruldu."),
        res.ok ? (start ? "ok" : "") : "err");
      if (!res.ok) toast(res.message, 4000);
      setCameraUI(!!res.running);
      state.running = !!res.running;
      if (state.running) startPreviewLoop();
      else stopPreviewLoop();
    } catch (err) {
      toast("Köprü hatası: " + err.message, 4000);
    }
  }

  /* ─────────────── yakalama ─────────────── */

  async function captureNow(fromShortcut = false) {
    if (!state.running) {
      setStatus("Yakalama için önce kamerayı başlatın.", "warn");
      return;
    }
    if (!fromShortcut && Date.now() < captureCooldown) return;
    captureCooldown = Date.now() + 250;

    const btn = $("btnCapture");
    btn.classList.add("flash");
    setTimeout(() => btn.classList.remove("flash"), 180);

    const flash = $("cameraFlash");
    if (flash) {
      flash.classList.add("flash-active");
      setTimeout(() => flash.classList.remove("flash-active"), 140);
    }

    const nameInput = $("filenameInput");
    const filename = nameInput ? nameInput.value.trim() : "";

    try {
      const res = await call("capture_now", filename);
      if (res.ok) {
        setStatus(res.message, "ok");
        $("countLabel").textContent = String(res.count || 0);
        $("lastLabel").textContent = res.name || "—";
        toast(res.message);
      } else {
        setStatus(res.message || "Yakalama başarısız.", "warn");
        toast(res.message || "Yakalama başarısız.", 3600);
      }
    } catch (err) {
      setStatus("Köprü hatası: " + err.message, "err");
    }
  }

  /* ─────────────── otomatik yakalama ─────────────── */

  function updateAutoUI() {
    $("autoToggle").checked = state.auto;
    $("intervalInput").value = state.interval;
  }

  function startAuto() {
    if (autoTimer) clearInterval(autoTimer);
    autoTimer = setInterval(async () => {
      if (!state.auto) return;
      if (autoBusy || !state.running) return;
      const iv = parseFloat($("intervalInput").value);
      if (!(iv > 0)) return;
      autoBusy = true;
      // yakalama zamanını kontrol etmek için son yakalama saatini takip et
      const now = Date.now();
      if (!state._lastAuto) state._lastAuto = 0;
      if (now - state._lastAuto >= iv * 1000) {
        state._lastAuto = now;
        const nameInput = $("filenameInput");
        const filename = nameInput ? nameInput.value.trim() : "";
        try { await call("capture_now", filename); }
        catch (e) { /* yut */ }
      }
      autoBusy = false;
    }, 400);
  }

  /* ─────────────── ayarlar paneli ─────────────── */

  function setSliderFill(el) {
    const min = parseFloat(el.min) || 0;
    const max = parseFloat(el.max) || 1;
    const v = parseFloat(el.value);
    const pct = ((v - min) / (max - min)) * 100;
    el.style.setProperty("--fill", pct + "%");
  }

  function updateQualityVisibility() {
    const isJpg = $("sFormat") && $("sFormat").value === "jpg";
    const fieldQuality = $("fieldQuality");
    if (fieldQuality) {
      fieldQuality.style.opacity = isJpg ? "1" : "0.45";
      if ($("sQuality")) $("sQuality").disabled = !isJpg;
    }
  }

  async function openSettings() {
    try {
      const camRes = await call("get_cameras");
      if (camRes && camRes.cameras) {
        populateCameras(camRes.cameras, state.cfg ? state.cfg.camera_index : 0);
      }
    } catch (_) {}
    const c = state.cfg;
    if (c) {
      if ($("sCamera")) $("sCamera").value = c.camera_index;
      if ($("sCamRes")) $("sCamRes").value = (c.cam_resolution || "auto").toLowerCase();
      $("sWidth").value = c.width;
      $("sHeight").value = c.height;
      $("sFormat").value = (c.image_format || "png").toLowerCase();
      if ($("sQuality")) {
        const q = c.jpeg_quality || 95;
        $("sQuality").value = q;
        if ($("sQualityVal")) $("sQualityVal").textContent = "%" + q;
        setSliderFill($("sQuality"));
      }
      if ($("sEdevlet")) {
        $("sEdevlet").checked = c.edevlet_limit !== false;
      }
      $("sDir").value = c.output_dir || "captures";
      $("sFace").checked = !!c.face_detect;
      $("sHead").value = c.face_head_ratio;
      $("sVoff").value = c.face_v_offset;
      $("sSmooth").value = c.face_smoothing;
      const ev = c.face_detect_every || 3;
      $("sEvery").value = ev;
      if ($("sEveryNum")) $("sEveryNum").value = ev;
      if ($("sEveryVal")) {
        let desc = "";
        if (ev === 1) desc = "1 kare (Ultra Akıcı)";
        else if (ev === 2) desc = "2 kare (Hızlı)";
        else if (ev === 3) desc = "3 kare (Önerilen)";
        else desc = `${ev} kare`;
        $("sEveryVal").textContent = desc;
      }
      $("sZoom").value = state.zoom;
      $("sHeadVal").textContent = Number(c.face_head_ratio).toFixed(2);
      $("sVoffVal").textContent = Number(c.face_v_offset).toFixed(2);
      $("sSmoothVal").textContent = Number(c.face_smoothing).toFixed(2);
      $("sZoomVal").textContent = Number(state.zoom).toFixed(1) + "x";
      [["sHead", "sHeadVal"], ["sVoff", "sVoffVal"], ["sSmooth", "sSmoothVal"], ["sZoom", "sZoomVal"], ["sEvery", "sEveryVal"]].forEach(
        ([s, v]) => { if ($(s)) setSliderFill($(s)); }
      );
      updateQualityVisibility();
    }
    $("settingsOverlay").classList.remove("hidden");
    setTimeout(() => $("sCamera").focus(), 60);
  }

  function closeSettings() {
    $("settingsOverlay").classList.add("hidden");
  }

  async function saveSettings() {
    const values = {
      camera_index: parseInt($("sCamera").value, 10) || 0,
      cam_resolution: $("sCamRes") ? $("sCamRes").value : "auto",
      width: parseInt($("sWidth").value, 10) || 133,
      height: parseInt($("sHeight").value, 10) || 171,
      image_format: $("sFormat").value,
      jpeg_quality: $("sQuality") ? (parseInt($("sQuality").value, 10) || 95) : 95,
      edevlet_limit: $("sEdevlet") ? $("sEdevlet").checked : true,
      output_dir: $("sDir").value.trim() || "captures",
      face_detect: $("sFace").checked,
      face_head_ratio: parseFloat($("sHead").value),
      face_v_offset: parseFloat($("sVoff").value),
      face_smoothing: parseFloat($("sSmooth").value),
      face_detect_every: parseInt($("sEveryNum") ? $("sEveryNum").value : $("sEvery").value, 10) || 3,
      frame_zoom: parseFloat($("sZoom").value) || state.zoom,
    };
    try {
      const res = await call("apply_settings", values);
      if (res.ok) {
        if (res.state) applyState(res.state);
        setStatus(res.message, "ok");
        toast(res.message);
        closeSettings();
        if (state.running) startPreviewLoop();
      } else {
        toast(res.message, 4200);
        setStatus(res.message, "err");
      }
    } catch (err) {
      toast("Köprü hatası: " + err.message, 4000);
    }
  }

  /* ─────────────── klasör & ikinci monitör ─────────────── */

  async function openFolder() {
    const res = await call("open_folder").catch(() => null);
    if (res && !res.ok) toast(res.message, 3000);
  }

  async function browseFolder() {
    const res = await call("pick_folder").catch(() => null);
    if (res && res.ok && res.path) {
      $("sDir").value = res.path;
    }
  }

  async function openTargetMonitorWindow() {
    try {
      const res = await call("open_target_window");
      if (res && !res.ok) {
        toast("Pencere açılırken hata: " + (res.message || "Bilinmeyen hata"), 4000);
      } else {
        setStatus("Hedef önizleme penceresi açıldı (ikinci monitör).", "ok");
      }
    } catch (err) {
      toast("Köprü hatası: " + err.message, 4000);
    }
  }

  /* ─────────────── olaylar ─────────────── */

  function onKey(e) {
    // metin alanında yazarken SPACE ve kısayolları yakalama
    const active = document.activeElement || {};
    const tag = active.tagName;
    if (["INPUT", "TEXTAREA", "SELECT"].includes(tag)) {
      if (e.key === "Escape") closeSettings();
      if (e.key === "Enter" && active.id === "filenameInput") {
        e.preventDefault();
        captureNow();
      }
      return;
    }
    if (e.code === "Space") {
      e.preventDefault();
      if (!$("settingsOverlay").classList.contains("hidden")) return;
      captureNow(true);
    } else if (e.key === "Escape") {
      if (!$("settingsOverlay").classList.contains("hidden")) closeSettings();
    } else if (e.key === "+" || e.key === "=") {
      e.preventDefault();
      setZoom(state.zoom + 0.1);
    } else if (e.key === "-" || e.key === "_") {
      e.preventDefault();
      setZoom(state.zoom - 0.1);
    } else if (e.key === "0") {
      e.preventDefault();
      setZoom(1.0);
    } else if (e.key === "f" || e.key === "F") {
      e.preventDefault();
      toggleFaceDetect();
    }
  }

  function wireUI() {
    $("btnStart").addEventListener("click", () => toggleCamera(true));
    $("btnStop").addEventListener("click", () => toggleCamera(false));
    $("btnCapture").addEventListener("click", () => captureNow());
    $("btnFolder").addEventListener("click", openFolder);
    $("btnSettings").addEventListener("click", openSettings);

    const btnOpenTarget = $("btnOpenTargetWindow");
    if (btnOpenTarget) {
      btnOpenTarget.addEventListener("click", openTargetMonitorWindow);
    }
    const btnToolbarMon = $("btnToolbarMonitor");
    if (btnToolbarMon) {
      btnToolbarMon.addEventListener("click", openTargetMonitorWindow);
    }

    // Hızlı Yüz Takibi Kontrolleri
    const btnQuickFace = $("btnQuickFace");
    if (btnQuickFace) {
      btnQuickFace.addEventListener("click", () => toggleFaceDetect());
    }
    const btnEveryDec = $("btnEveryDec");
    if (btnEveryDec) {
      btnEveryDec.addEventListener("click", () => {
        const cur = state.cfg ? state.cfg.face_detect_every : 3;
        setFaceEvery(cur - 1);
      });
    }
    const btnEveryInc = $("btnEveryInc");
    if (btnEveryInc) {
      btnEveryInc.addEventListener("click", () => {
        const cur = state.cfg ? state.cfg.face_detect_every : 3;
        setFaceEvery(cur + 1);
      });
    }
    document.querySelectorAll(".freq-chip").forEach((chip) => {
      chip.addEventListener("click", () => {
        const val = parseInt(chip.getAttribute("data-every"), 10);
        if (val) setFaceEvery(val);
      });
    });

    const sFace = $("sFace");
    if (sFace) {
      sFace.addEventListener("change", (e) => {
        toggleFaceDetect(e.target.checked);
      });
    }

    const sEvery = $("sEvery");
    const sEveryNum = $("sEveryNum");
    if (sEvery) {
      sEvery.addEventListener("input", (e) => {
        const val = parseInt(e.target.value, 10);
        if (sEveryNum) sEveryNum.value = val;
        setFaceEvery(val);
      });
    }
    if (sEveryNum) {
      sEveryNum.addEventListener("input", (e) => {
        const val = parseInt(e.target.value, 10);
        if (val >= 1) {
          if (sEvery) sEvery.value = val;
          setFaceEvery(val);
        }
      });
    }

    // Yakınlaştırma kontrolleri
    const zoomSlider = $("zoomSlider");
    if (zoomSlider) {
      zoomSlider.addEventListener("mousedown", () => { state.sliderDragging = true; });
      zoomSlider.addEventListener("touchstart", () => { state.sliderDragging = true; }, { passive: true });
      window.addEventListener("mouseup", () => { state.sliderDragging = false; });
      window.addEventListener("touchend", () => { state.sliderDragging = false; });
      zoomSlider.addEventListener("input", (e) => {
        setZoom(parseFloat(e.target.value));
      });
    }

    if ($("btnZoomIn")) {
      $("btnZoomIn").addEventListener("click", () => setZoom(state.zoom + 0.1));
    }
    if ($("btnZoomOut")) {
      $("btnZoomOut").addEventListener("click", () => setZoom(state.zoom - 0.1));
    }
    if ($("btnZoomReset")) {
      $("btnZoomReset").addEventListener("click", () => setZoom(1.0));
    }

    const liveStage = $("liveStage");
    if (liveStage) {
      liveStage.addEventListener("wheel", (e) => {
        e.preventDefault();
        const delta = e.deltaY < 0 ? 0.05 : -0.05;
        setZoom(state.zoom + delta);
      }, { passive: false });
    }

    const sZoom = $("sZoom");
    if (sZoom) {
      sZoom.addEventListener("input", (e) => {
        const v = parseFloat(e.target.value);
        if ($("sZoomVal")) $("sZoomVal").textContent = v.toFixed(1) + "x";
        setSliderFill(e.target);
        setZoom(v);
      });
    }

    $("autoToggle").addEventListener("change", (e) => {
      state.auto = e.target.checked;
      localStorage.setItem("wc_auto", String(state.auto));
      if (state.auto) state._lastAuto = 0;
    });
    $("intervalInput").addEventListener("input", (e) => {
      const v = parseFloat(e.target.value);
      if (v > 0) {
        state.interval = v;
        localStorage.setItem("wc_interval", String(v));
      }
    });

    ["sHead", "sVoff", "sSmooth"].forEach((id) => {
      $(id).addEventListener("input", () => {
        const v = parseFloat($(id).value).toFixed(2);
        $(id + "Val").textContent = v;
        setSliderFill($(id));
      });
    });

    if ($("sQuality")) {
      $("sQuality").addEventListener("input", () => {
        if ($("sQualityVal")) $("sQualityVal").textContent = "%" + $("sQuality").value;
        setSliderFill($("sQuality"));
      });
    }

    if ($("sFormat")) {
      $("sFormat").addEventListener("change", updateQualityVisibility);
    }

    $("sCancel").addEventListener("click", closeSettings);
    $("sSave").addEventListener("click", saveSettings);
    $("sDirBrowse").addEventListener("click", browseFolder);

    const nameInput = $("filenameInput");
    const nameClear = $("filenameClear");
    if (nameInput && nameClear) {
      nameInput.addEventListener("input", () => {
        nameClear.classList.toggle("hidden", !nameInput.value.trim());
      });
      nameClear.addEventListener("click", () => {
        nameInput.value = "";
        nameClear.classList.add("hidden");
        nameInput.focus();
      });
    }

    const dotClose = document.querySelector(".dot-close");
    if (dotClose) {
      dotClose.addEventListener("click", () => {
        try { call("exit_app"); } catch (_) { window.close(); }
      });
    }

    document.addEventListener("keydown", onKey);
    $("settingsOverlay").addEventListener("click", (e) => {
      if (e.target === $("settingsOverlay")) closeSettings();
    });

    window.addEventListener("beforeunload", () => {
      try { call("stop_camera"); } catch (_) {}
      try { call("exit_app"); } catch (_) {}
    });
  }

  /* ─────────────── başlat ─────────────── */

  async function init() {
    try {
      const st = await call("get_state");
      applyState(st);
      startPreviewLoop(); // döngü, çalışma durumunu kendi kontrol eder
      if (!st.running) {
        toggleCamera(true);
      }
    } catch (err) {
      setStatus("Köprü hatası: " + err.message, "err");
    }
  }

  function boot() {
    wireUI();
    updateAutoUI();
    startAuto();

    if (hasBridge()) {
      init();
      return;
    }

    // Köprü henüz enjekte edilmedi; pywebview hazır olduğunda
    // "pywebviewready" olayını tetikler — bu olayı bekliyoruz.
    setStatus("Köprü bekleniyor…");
    window.addEventListener("pywebviewready", () => init(), { once: true });

    // Köprü hiç gelmezse (ör. index.html doğrudan bir tarayıcıda açıldıysa)
    // makul bir süre sonra uyarı göster.
    setTimeout(() => {
      if (!hasBridge()) {
        setStatus("pywebview köprüsü yok — lütfen python webapp.py ile çalıştırın.", "err");
      }
    }, 4000);
  }

  document.addEventListener("DOMContentLoaded", boot);
})();
