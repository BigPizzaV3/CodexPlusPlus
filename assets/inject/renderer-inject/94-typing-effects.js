  // 特效只观察原生编辑事件；不写输入框、不改选区，也不触发 React 输入事务。
  window.__codexPlusTypingEffectsRuntime?.dispose?.();
  const codexPlusTypingEffects = {
    installed: false, disposed: false, mode: "off", canvas: null, context: null,
    bounds: null, particles: [], frame: null, frameAt: 0, lastBurstAt: -Infinity,
    editor: null, composition: null, compositionFrame: null, media: null,
  };
  const codexPlusTypingEffectMaxParticles = 180;
  const codexPlusTypingEffectInterval = 40;
  const codexPlusTypingEffectOwner = "builtin-typing-effects";
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-typing-effects"]');

  function codexPlusTypingEffectMode() {
    const value = codexPlusSettings().typingEffect;
    return ["rainbow", "fireworks", "stars"].includes(value) ? value : "off";
  }

  function clearCodexPlusTypingEffects(resetComposition = true) {
    const state = codexPlusTypingEffects;
    if (state.frame !== null) cancelAnimationFrame(state.frame);
    state.frame = null;
    state.canvas?.remove();
    state.canvas = state.context = state.bounds = state.editor = null;
    if (resetComposition !== false) {
      state.composition = null;
      if (state.compositionFrame !== null) cancelAnimationFrame(state.compositionFrame);
      state.compositionFrame = null;
    }
    state.particles.length = 0;
    state.frameAt = 0;
    state.lastBurstAt = -Infinity;
  }

  function syncCodexPlusTypingEffects() {
    const state = codexPlusTypingEffects;
    const mode = codexPlusTypingEffectMode();
    if (state.mode !== mode || mode === "off" || state.media?.matches || document.hidden) clearCodexPlusTypingEffects();
    state.mode = mode;
  }

  function codexPlusTypingEffectEditor(target) {
    const editor = target?.closest?.('.ProseMirror[contenteditable="true"], textarea, [role="textbox"][contenteditable="true"]');
    if (!editor?.isConnected || editor.matches(':disabled, [readonly], [contenteditable="false"]')
        || isExtensionUiNode(editor) || editor.closest('[inert], [aria-hidden="true"]') || !visibleElement(editor)) return null;
    // 新版有明确 composer 标记；旧版必须与 composer-footer 同属一个局部容器。
    let composer = editor.closest('[data-codex-composer-root], [data-composer-surface-variant]');
    if (!composer) {
      let parent = editor.parentElement;
      for (let depth = 0; parent && parent !== document.body && depth < 4; depth++, parent = parent.parentElement) {
        if (parent.querySelector?.('.composer-footer')) { composer = parent; break; }
        if (parent.tagName === "FORM") break;
      }
    }
    return composer && (document.activeElement === editor || editor.contains(document.activeElement)) ? editor : null;
  }

  function codexPlusTypingEffectText(editor) {
    return editor instanceof HTMLTextAreaElement ? editor.value : editor.textContent || "";
  }

  function codexPlusTypingEffectTextareaCaret(editor) {
    const style = getComputedStyle(editor);
    const rect = editor.getBoundingClientRect();
    const width = editor.offsetWidth || rect.width;
    const height = editor.offsetHeight || rect.height;
    // 镜像不显示滚动条，须扣除原生 textarea 的滚动条占用，保持实际换行列宽。
    const mirrorWidth = editor.clientWidth > 0
      ? editor.clientWidth + (parseFloat(style.borderLeftWidth) || 0) + (parseFloat(style.borderRightWidth) || 0) : width;
    const mirror = document.createElement("div");
    mirror.setAttribute("data-codex-plus-ext", codexPlusTypingEffectOwner);
    mirror.setAttribute("aria-hidden", "true");
    // offsetWidth 使用布局尺寸，最终按真实 rect 缩放，兼容编辑器祖先上的 CSS zoom。
    const properties = ["direction", "fontFamily", "fontSize", "fontWeight", "fontStyle", "fontVariant", "fontStretch",
      "fontKerning", "fontFeatureSettings", "fontVariationSettings", "lineHeight", "letterSpacing", "wordSpacing", "textTransform", "textIndent", "textAlign", "tabSize", "wordBreak",
      "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "borderTopWidth", "borderRightWidth", "borderBottomWidth", "borderLeftWidth"];
    for (const property of properties) mirror.style[property] = style[property];
    Object.assign(mirror.style, {
      position: "fixed", left: "-100000px", top: "0", visibility: "hidden", pointerEvents: "none",
      boxSizing: "border-box", width: `${mirrorWidth}px`, height: "auto", minHeight: "0", maxHeight: "none",
      borderStyle: "solid", overflow: "hidden", whiteSpace: editor.wrap === "off" ? "pre" : "pre-wrap",
      overflowWrap: editor.wrap === "off" ? "normal" : "break-word", zoom: "1",
    });
    const position = editor.selectionEnd ?? editor.value.length;
    mirror.textContent = editor.value.slice(0, position);
    const marker = document.createElement("span");
    marker.textContent = editor.value.slice(position) || "\u200b";
    mirror.appendChild(marker);
    document.body.appendChild(mirror);
    try {
      const mirrorRect = mirror.getBoundingClientRect();
      const markerRect = marker.getClientRects()[0] || marker.getBoundingClientRect();
      const mirrorScale = mirrorRect.width / mirrorWidth || 1;
      const scaleX = rect.width / width || 1;
      const scaleY = rect.height / height || 1;
      const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.25 || 20;
      return {
        x: rect.left + ((markerRect.left - mirrorRect.left) / mirrorScale - editor.scrollLeft) * scaleX,
        y: rect.top + ((markerRect.top - mirrorRect.top) / mirrorScale - editor.scrollTop + lineHeight * .55) * scaleY,
      };
    } finally { mirror.remove(); }
  }

  function codexPlusTypingEffectCaret(editor) {
    const box = editor.getBoundingClientRect();
    let point;
    if (editor instanceof HTMLTextAreaElement) point = codexPlusTypingEffectTextareaCaret(editor);
    else {
      const selection = window.getSelection?.();
      if (!selection?.rangeCount || !editor.contains(selection.anchorNode) || !editor.contains(selection.focusNode)) return null;
      const range = selection.getRangeAt(0).cloneRange();
      range.collapse(false);
      let rect = range.getClientRects()[0] || range.getBoundingClientRect();
      let precedingCharacter = false;
      if (!rect?.height && range.endContainer?.nodeType === 3 && range.endOffset > 0) {
        range.setStart(range.endContainer, range.endOffset - 1);
        rect = range.getBoundingClientRect();
        precedingCharacter = true;
      }
      const style = getComputedStyle(editor);
      const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.25 || 20;
      if (rect?.height) point = {
        x: precedingCharacter && style.direction !== "rtl" ? rect.right : rect.left,
        y: rect.top + rect.height * .55,
      };
      else {
        // 空段落/BR 的 collapsed Range 可能没有几何，使用所在行而非整个 editor 的首行。
        let node = range.endContainer?.nodeType === 1 ? range.endContainer : range.endContainer?.parentElement;
        const child = node?.childNodes?.[range.endOffset] || node?.childNodes?.[Math.max(0, range.endOffset - 1)];
        if (child?.nodeType === 1) node = child;
        let fallback = box;
        let fallbackStyle = style;
        for (let depth = 0; node && editor.contains(node) && depth < 4; depth++, node = node.parentElement) {
          const nodeRect = node.getBoundingClientRect?.();
          if (nodeRect?.height > 0) { fallback = nodeRect; fallbackStyle = getComputedStyle(node); break; }
        }
        const scaleX = box.width / (editor.offsetWidth || box.width) || 1;
        const scaleY = box.height / (editor.offsetHeight || box.height) || 1;
        const fallbackLineHeight = parseFloat(fallbackStyle.lineHeight) || parseFloat(fallbackStyle.fontSize) * 1.25 || lineHeight;
        point = {
          x: fallback.left + (parseFloat(fallbackStyle.paddingLeft) || 0) * scaleX,
          y: fallback.top + ((parseFloat(fallbackStyle.paddingTop) || 0) + fallbackLineHeight * .55) * scaleY,
        };
      }
    }
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) return null;
    return { x: Math.max(box.left, Math.min(box.right, point.x)), y: Math.max(box.top, Math.min(box.bottom, point.y)) };
  }

  function createCodexPlusTypingEffectCanvas() {
    const state = codexPlusTypingEffects;
    if (state.canvas?.isConnected) return true;
    const canvas = document.createElement("canvas");
    canvas.setAttribute("data-codex-plus-ext", codexPlusTypingEffectOwner);
    canvas.setAttribute("aria-hidden", "true");
    Object.assign(canvas.style, {
      position: "fixed", top: "0", left: "0", width: "100%", height: "100%", pointerEvents: "none",
      zIndex: "2147483000", background: "transparent",
    });
    const context = canvas.getContext("2d");
    if (!context) return false;
    document.documentElement.appendChild(canvas);
    const bounds = canvas.getBoundingClientRect();
    const ratio = Math.min(2, Math.max(1, window.devicePixelRatio || 1));
    canvas.width = Math.max(1, Math.round(bounds.width * ratio));
    canvas.height = Math.max(1, Math.round(bounds.height * ratio));
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    Object.assign(state, { canvas, context, bounds });
    return true;
  }

  function drawCodexPlusTypingEffects(timestamp) {
    const state = codexPlusTypingEffects;
    state.frame = null;
    if (state.disposed || state.mode !== codexPlusTypingEffectMode() || state.media?.matches || document.hidden
        || !state.editor?.isConnected || codexPlusTypingEffectEditor(document.activeElement) !== state.editor || !state.canvas?.isConnected) {
      clearCodexPlusTypingEffects(); return;
    }
    const elapsed = state.frameAt ? Math.max(0, Math.min(32, timestamp - state.frameAt)) : 16;
    state.frameAt = timestamp;
    const context = state.context;
    context.clearRect(0, 0, state.bounds.width, state.bounds.height);
    state.particles = state.particles.filter((particle) => {
      particle.age += elapsed;
      if (particle.age >= particle.life) return false;
      particle.x += particle.vx * elapsed / 1000;
      particle.y += particle.vy * elapsed / 1000;
      particle.vy += particle.gravity * elapsed / 1000;
      const alpha = (1 - particle.age / particle.life) ** 1.5;
      context.save();
      context.globalAlpha = alpha;
      context.fillStyle = particle.color;
      context.translate(particle.x - state.bounds.left, particle.y - state.bounds.top);
      context.rotate(particle.rotation + particle.age / 700);
      if (particle.star) {
        context.beginPath();
        for (let index = 0; index < 10; index++) {
          const radius = particle.size * (index % 2 ? .42 : 1);
          const angle = index * Math.PI / 5 - Math.PI / 2;
          if (index === 0) context.moveTo(Math.cos(angle) * radius, Math.sin(angle) * radius);
          else context.lineTo(Math.cos(angle) * radius, Math.sin(angle) * radius);
        }
        context.closePath(); context.fill();
      } else context.fillRect(-particle.size / 2, -particle.size / 2, particle.size, particle.size * 1.7);
      context.restore();
      return true;
    });
    if (state.particles.length) state.frame = requestAnimationFrame(drawCodexPlusTypingEffects);
    else clearCodexPlusTypingEffects(false);
  }

  function emitCodexPlusTypingEffects(editor) {
    const state = codexPlusTypingEffects;
    syncCodexPlusTypingEffects();
    if (state.disposed || state.mode === "off" || state.media?.matches || document.hidden) return false;
    const now = performance.now();
    if (now - state.lastBurstAt < codexPlusTypingEffectInterval) return false;
    const point = codexPlusTypingEffectCaret(editor);
    if (!point || !createCodexPlusTypingEffectCanvas()) return false;
    state.lastBurstAt = now;
    state.editor = editor;
    const count = state.mode === "fireworks" ? 26 : state.mode === "stars" ? 10 : 16;
    const available = Math.max(0, codexPlusTypingEffectMaxParticles - state.particles.length);
    if (!available) return false;
    const hue = now / 9 % 360;
    for (let index = 0; index < Math.min(count, available); index++) {
      const angle = state.mode === "fireworks" ? index / count * Math.PI * 2 : -Math.PI + Math.random() * Math.PI;
      const speed = state.mode === "fireworks" ? 65 + Math.random() * 100 : 35 + Math.random() * 95;
      state.particles.push({
        x: point.x, y: point.y, vx: Math.cos(angle) * speed, vy: Math.sin(angle) * speed,
        gravity: state.mode === "stars" ? 95 : 180, age: 0, life: 420 + Math.random() * 360,
        size: state.mode === "stars" ? 3 + Math.random() * 3 : 1.5 + Math.random() * 2.2,
        star: state.mode === "stars", rotation: Math.random() * Math.PI,
        color: `hsl(${state.mode === "stars" ? 38 + Math.random() * 22 : hue + index * 360 / count} 95% 68%)`,
      });
    }
    if (state.particles.length && state.frame === null) state.frame = requestAnimationFrame(drawCodexPlusTypingEffects);
    return true;
  }

  function createCodexPlusTypingEffectComposition(editor, before = codexPlusTypingEffectText(editor)) {
    return { editor, before, ended: false, emitted: false, lastEmittedText: null, lastInputText: before, sawTrustedInput: false, attempts: 0 };
  }

  function commitCodexPlusTypingEffectComposition(composition) {
    const state = codexPlusTypingEffects;
    if (state.composition !== composition || !composition.data || !composition.trusted
        || !codexPlusTypingEffectEditor(composition.editor)) return false;
    const text = codexPlusTypingEffectText(composition.editor);
    if (composition.emitted && text === composition.committed) return true;
    // 最终 input 通常紧接组词 input；同一份已绘制文本不重复发射。
    if (text !== composition.lastEmittedText) {
      if (text === composition.before || !emitCodexPlusTypingEffects(composition.editor)) return false;
    }
    composition.emitted = true;
    composition.committed = text;
    composition.lastEmittedText = text;
    return true;
  }

  function updateCodexPlusTypingEffectComposition(composition) {
    const state = codexPlusTypingEffects;
    if (state.composition !== composition || state.disposed || !codexPlusTypingEffectEditor(composition.editor)) return;
    let emitted;
    if (composition.ended) emitted = commitCodexPlusTypingEffectComposition(composition);
    else {
      const text = codexPlusTypingEffectText(composition.editor);
      emitted = text === composition.lastEmittedText || (composition.lastInputData === "" && !composition.backwardDeletion);
      if (!emitted && (text !== composition.before || composition.backwardDeletion) && emitCodexPlusTypingEffects(composition.editor)) {
        composition.lastEmittedText = text;
        emitted = true;
      }
    }
    if (emitted) {
      if (state.compositionFrame !== null) cancelAnimationFrame(state.compositionFrame);
      state.compositionFrame = null;
      composition.attempts = 0;
    } else if (state.composition === composition && state.compositionFrame === null && composition.attempts < 3) {
      // 输入法/编辑器可能在事件之后才恢复选区；只做有界补发，不开启常驻扫描。
      state.compositionFrame = requestAnimationFrame(() => {
        state.compositionFrame = null;
        composition.attempts += 1;
        updateCodexPlusTypingEffectComposition(composition);
      });
    }
  }

  function codexPlusTypingEffectInput(event) {
    if (event.isTrusted !== true) return;
    const backwardDeletion = ["deleteContentBackward", "deleteWordBackward", "deleteSoftLineBackward", "deleteHardLineBackward"].includes(event.inputType);
    if (!backwardDeletion && !["insertText", "insertLineBreak", "insertParagraph", "insertCompositionText", "insertFromComposition"].includes(event.inputType)) return;
    const editor = codexPlusTypingEffectEditor(event.target);
    if (!editor) return;
    const state = codexPlusTypingEffects;
    let composition = state.composition;
    const compositionInput = event.isComposing || ["insertCompositionText", "insertFromComposition"].includes(event.inputType);
    if (compositionInput && (!composition || composition.editor !== editor
        || (composition.ended && composition.emitted && (event.isComposing || codexPlusTypingEffectText(editor) !== composition.committed)))) {
      // 部分输入法不会派发 compositionstart，但仍有可信的原生组词 input。
      if (state.compositionFrame !== null) cancelAnimationFrame(state.compositionFrame);
      state.compositionFrame = null;
      composition = state.composition = createCodexPlusTypingEffectComposition(editor, null);
      if (event.inputType === "insertFromComposition") Object.assign(composition, { ended: true, data: event.data, trusted: true });
    }
    if (composition?.editor === editor) {
      if (backwardDeletion && composition.ended && !event.isComposing) {
        if (state.compositionFrame !== null) cancelAnimationFrame(state.compositionFrame);
        state.compositionFrame = null;
        state.composition = null;
        emitCodexPlusTypingEffects(editor);
        return;
      }
      if (!composition.ended || !composition.emitted) {
        const text = codexPlusTypingEffectText(editor);
        // 输入法退格也可能沿用 insertCompositionText，且清空组词时 data 为空。
        const shorterPreedit = compositionInput && typeof composition.lastInputText === "string" && text.length < composition.lastInputText.length;
        Object.assign(composition, {
          sawTrustedInput: true, lastInputData: event.data, lastInputText: text,
          backwardDeletion: backwardDeletion || shorterPreedit, attempts: 0,
        });
        updateCodexPlusTypingEffectComposition(composition);
        return;
      }
      if (codexPlusTypingEffectText(editor) === composition.committed) return;
      state.composition = null;
    }
    if (event.isComposing || (!backwardDeletion && !["insertText", "insertLineBreak", "insertParagraph"].includes(event.inputType))) return;
    if (event.inputType === "insertText" && !event.data) return;
    emitCodexPlusTypingEffects(editor);
  }

  function codexPlusTypingEffectCompositionStart(event) {
    if (event.isTrusted !== true) return;
    syncCodexPlusTypingEffects();
    if (codexPlusTypingEffects.mode === "off" || codexPlusTypingEffects.media?.matches || document.hidden) return;
    const editor = codexPlusTypingEffectEditor(event.target);
    if (editor) {
      if (codexPlusTypingEffects.compositionFrame !== null) cancelAnimationFrame(codexPlusTypingEffects.compositionFrame);
      codexPlusTypingEffects.compositionFrame = null;
      codexPlusTypingEffects.composition = createCodexPlusTypingEffectComposition(editor);
    }
  }

  function codexPlusTypingEffectCompositionEnd(event) {
    const composition = codexPlusTypingEffects.composition;
    if (!composition || composition.editor !== codexPlusTypingEffectEditor(event.target)) return;
    // 有些输入法/宿主转发结束事件，isTrusted 为 false；仅接纳之前已观察到的可信组词内容。
    if (event.isTrusted !== true && (!composition.sawTrustedInput || (event.data && event.data !== composition.lastInputData))) return;
    if (!event.data) {
      codexPlusTypingEffects.composition = null;
      if (codexPlusTypingEffects.compositionFrame !== null) cancelAnimationFrame(codexPlusTypingEffects.compositionFrame);
      codexPlusTypingEffects.compositionFrame = null;
      return;
    }
    Object.assign(composition, { ended: true, data: event.data, trusted: event.isTrusted === true || composition.sawTrustedInput });
    queueMicrotask(() => {
      updateCodexPlusTypingEffectComposition(composition);
    });
  }

  function codexPlusTypingEffectFocusOut(event) {
    const state = codexPlusTypingEffects;
    if (event.target === state.editor || event.target === state.composition?.editor) clearCodexPlusTypingEffects();
  }

  function installCodexPlusTypingEffects() {
    const state = codexPlusTypingEffects;
    if (state.disposed) return;
    if (!state.installed) {
      state.installed = true;
      state.media = window.matchMedia?.("(prefers-reduced-motion: reduce)") || null;
      document.addEventListener("input", codexPlusTypingEffectInput, true);
      document.addEventListener("compositionstart", codexPlusTypingEffectCompositionStart, true);
      document.addEventListener("compositionend", codexPlusTypingEffectCompositionEnd, true);
      document.addEventListener("focusout", codexPlusTypingEffectFocusOut, true);
      document.addEventListener("visibilitychange", syncCodexPlusTypingEffects);
      window.addEventListener("blur", clearCodexPlusTypingEffects);
      window.addEventListener("resize", clearCodexPlusTypingEffects);
      state.media?.addEventListener?.("change", syncCodexPlusTypingEffects);
    }
    syncCodexPlusTypingEffects();
  }

  window.__codexPlusTypingEffectsRuntime = {
    dispose() {
      const state = codexPlusTypingEffects;
      state.disposed = true;
      clearCodexPlusTypingEffects();
      if (!state.installed) return;
      document.removeEventListener("input", codexPlusTypingEffectInput, true);
      document.removeEventListener("compositionstart", codexPlusTypingEffectCompositionStart, true);
      document.removeEventListener("compositionend", codexPlusTypingEffectCompositionEnd, true);
      document.removeEventListener("focusout", codexPlusTypingEffectFocusOut, true);
      document.removeEventListener("visibilitychange", syncCodexPlusTypingEffects);
      window.removeEventListener("blur", clearCodexPlusTypingEffects);
      window.removeEventListener("resize", clearCodexPlusTypingEffects);
      state.media?.removeEventListener?.("change", syncCodexPlusTypingEffects);
      state.installed = false;
    },
  };
