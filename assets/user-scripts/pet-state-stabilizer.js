/*
@codex-plus-script
name: Pet State Stabilizer
description: 通过 Codex++ 桥接把稳定器注入宠物 overlay，不再需要外部 Node / CDP WebSocket。
version: 4.2.0
author: Codex++ Community
*/

(() => {
  const api = window.codexPlus;
  if (!api || typeof api.call !== "function") {
    console.warn("[pet-stab] window.codexPlus 不可用，请升级 Codex++");
    return;
  }

  const VERSION = "4.2.0";
  const POLL_MS = 5000;

  const INJECT_CODE = String.raw`
(function(){
  if (window.__petStabDaemon && window.__petStabDaemon.destroy) {
    try { window.__petStabDaemon.destroy(); } catch(e) {}
  }
  var PET_ATTR = "data-codex-pet-state";
  var el = document.querySelector("[" + PET_ATTR + "]");
  var obs = null;
  var raf = null;
  var corrections = 0;

  var ROW_RUNNING = 7;
  var ROW_IDLE   = 0;
  var ROW_FAILED = 5;
  var ROW_LOOK_A = 9;
  var ROW_LOOK_B = 10;

  function destroy() {
    if (obs) { try { obs.disconnect(); } catch(e) {} obs = null; }
    if (raf) { cancelAnimationFrame(raf); raf = null; }
    try { el && el.style.removeProperty("background-position-y"); } catch(e) {}
  }

  function getState() { return el ? el.getAttribute(PET_ATTR) : null; }
  function getY() {
    if (!el) return null;
    var v = el.style.getPropertyValue("background-position-y");
    var m = v && v.match(/([-\d.]+)%/);
    return m ? parseFloat(m[1]) : null;
  }
  function setY(rowIdx) { el.style.setProperty("background-position-y", (rowIdx * 10) + "%"); }
  function getX() {
    if (!el) return null;
    var v = el.style.getPropertyValue("background-position-x");
    var m = v && v.match(/([-\d.]+)%/);
    return m ? parseFloat(m[1]) : null;
  }

  function computeLookRow(mouseX, mouseY) {
    if (!el) return null;
    var rect = el.getBoundingClientRect();
    if (rect.width === 0) return null;
    var dx = mouseX - (rect.left + rect.width / 2);
    var dy = mouseY - (rect.top + rect.height / 2);
    if (Math.hypot(dx, dy) < 1) return null;
    var dir = Math.round((Math.atan2(dx, -dy) * 180 / Math.PI + 360) % 360 / 22.5) % 16;
    return { row: dir < 8 ? ROW_LOOK_A : ROW_LOOK_B, col: dir % 8 };
  }

  function applyLook(mouseX, mouseY) {
    var look = computeLookRow(mouseX, mouseY);
    if (!look) return;
    window.__lookFrame = look;
    var colX = look.col / 7 * 100;
    el.style.setProperty("background-position-x", colX + "%");
    el.style.setProperty("background-position-y", (look.row * 10) + "%");
  }

  var mouseOverPet = false;
  var lastMouseX = 0, lastMouseY = 0;
  var lookUpdates = 0;

  if (el) {
    window.addEventListener("pointermove", function(e) {
      lastMouseX = e.clientX;
      lastMouseY = e.clientY;
      var rect = el.getBoundingClientRect();
      mouseOverPet = rect.width > 0 &&
        e.clientX >= rect.left && e.clientX <= rect.right &&
        e.clientY >= rect.top && e.clientY <= rect.bottom;
    }, true);

    obs = new MutationObserver(function() {
      if (!el) return;
      var state = getState();
      var y = getY();
      if (y == null) return;

      if (state === "running") {
        if (y !== 70) {
          setY(ROW_RUNNING);
          corrections++;
        }
      } else if (state === "failed") {
        if (y !== 50) {
          setY(ROW_FAILED);
          corrections++;
        }
      } else if (state !== "running" && state !== "failed" && !mouseOverPet && window.__lookFrame) {
        var wantY = (window.__lookFrame.row * 10) + "%";
        var wantX = (window.__lookFrame.col / 7 * 100) + "%";
        if (el.style.getPropertyValue("background-position-y") !== wantY) el.style.setProperty("background-position-y", wantY);
        if (el.style.getPropertyValue("background-position-x") !== wantX) el.style.setProperty("background-position-x", wantX);
      }
    });
    obs.observe(el, { attributes: true, attributeFilter: ["style"] });

    function trackTick() {
      if (!el || !document.contains(el)) { raf = null; return; }
      var state = getState();
      if (state !== "running" && state !== "failed" && !mouseOverPet) {
        applyLook(lastMouseX, lastMouseY);
        lookUpdates++;
      }
      raf = requestAnimationFrame(trackTick);
    }
    raf = requestAnimationFrame(trackTick);
  }

  window.__petStabDaemon = {
    version: "4.2.0",
    destroy: destroy,
    status: function() {
      return {
        state: getState(),
        y: getY(),
        x: getX(),
        corrections: corrections,
        mouseOverPet: mouseOverPet,
        lookUpdates: lookUpdates,
        tracking: !mouseOverPet && getState() !== "running" && getState() !== "failed",
      };
    }
  };
  console.info("[pet-stab-daemon] loaded v4.2.0");
})();
`;

  const STATUS_CODE = 'JSON.stringify({has: !!(window.__petStabDaemon), version: window.__petStabDaemon && window.__petStabDaemon.version})';

  let stopped = false;
  let timer = null;
  let injecting = false;

  function needsInject(result) {
    if (!result || result.ok === false) return true;
    try {
      const value = typeof result.value === "string" ? JSON.parse(result.value) : result.value;
      return !(value && value.has && value.version === VERSION);
    } catch (_) {
      return true;
    }
  }

  async function ensure() {
    if (stopped || injecting) return;
    injecting = true;
    try {
      const listed = await api.call("/cdp/targets");
      const overlays = (listed?.targets || []).filter((target) => target.kind === "avatar-overlay");
      if (!overlays.length) return;
      const status = await api.call("/cdp/evaluate", {
        kind: "avatar-overlay",
        expression: STATUS_CODE,
      });
      const results = status?.results || [];
      if (results.length && !results.some(needsInject)) return;
      await api.call("/cdp/evaluate", {
        kind: "avatar-overlay",
        expression: INJECT_CODE,
      });
    } catch (error) {
      api.fail?.(error);
    } finally {
      injecting = false;
    }
  }

  api.onCleanup?.(() => {
    stopped = true;
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
  });

  timer = setInterval(ensure, POLL_MS);
  ensure();
  console.info("[pet-stab] extension loaded v4.2.0");
})();
