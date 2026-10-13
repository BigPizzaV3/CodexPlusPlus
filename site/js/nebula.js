/* ============================================================
   Codex++ 官网 · Three.js 极光流体叠层
   单 pass 片元着色器：domain-warped fbm，青→靛→紫→琥珀色带
   用法：CodexNebula.attach(canvas, { intensity })
   ============================================================ */

(function () {
  "use strict";

  var REDUCED = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  var VERT = [
    "varying vec2 vUv;",
    "void main() {",
    "  vUv = uv;",
    "  gl_Position = vec4(position.xy, 0.0, 1.0);",
    "}"
  ].join("\n");

  var FRAG = [
    "precision highp float;",
    "varying vec2 vUv;",
    "uniform float uTime;",
    "uniform vec2  uRes;",
    "uniform vec2  uMouse;",
    "uniform float uIntensity;",
    "uniform float uSeed;",
    "",
    "float hash(vec2 p) {",
    "  p = fract(p * vec2(123.34, 456.21));",
    "  p += dot(p, p + 45.32);",
    "  return fract(p.x * p.y);",
    "}",
    "",
    "float noise(vec2 p) {",
    "  vec2 i = floor(p);",
    "  vec2 f = fract(p);",
    "  vec2 u = f * f * (3.0 - 2.0 * f);",
    "  float a = hash(i);",
    "  float b = hash(i + vec2(1.0, 0.0));",
    "  float c = hash(i + vec2(0.0, 1.0));",
    "  float d = hash(i + vec2(1.0, 1.0));",
    "  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);",
    "}",
    "",
    "float fbm(vec2 p) {",
    "  float v = 0.0;",
    "  float amp = 0.55;",
    "  mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);",
    "  for (int i = 0; i < 5; i++) {",
    "    v += amp * noise(p);",
    "    p = rot * p * 2.02 + vec2(3.1, 1.7);",
    "    amp *= 0.52;",
    "  }",
    "  return v;",
    "}",
    "",
    "vec3 palette(float t) {",
    "  vec3 c1 = vec3(0.08, 0.30, 0.55);",   // 深青
    "  vec3 c2 = vec3(0.30, 0.36, 0.95);",   // 靛
    "  vec3 c3 = vec3(0.58, 0.42, 0.98);",   // 紫
    "  vec3 c4 = vec3(0.95, 0.62, 0.32);",   // 琥珀
    "  vec3 col = mix(c1, c2, smoothstep(0.0, 0.45, t));",
    "  col = mix(col, c3, smoothstep(0.38, 0.78, t));",
    "  col = mix(col, c4, smoothstep(0.82, 1.0, t) * 0.55);",
    "  return col;",
    "}",
    "",
    "void main() {",
    "  vec2 uv = vUv;",
    "  vec2 asp = vec2(uRes.x / uRes.y, 1.0);",
    "  vec2 p = (uv - 0.5) * asp * 2.2;",
    "  float t = uTime * 0.05 + uSeed;",
    "",
    "  // 鼠标引力场",
    "  vec2 m = (uMouse - 0.5) * asp * 2.2;",
    "  float md = length(p - m);",
    "  p += 0.22 * (p - m) * exp(-md * md * 2.4);",
    "",
    "  // 双重 domain warp",
    "  vec2 q = vec2(fbm(p + vec2(0.0, t)), fbm(p + vec2(5.2, 1.3 - t)));",
    "  vec2 r = vec2(fbm(p + 2.6 * q + vec2(1.7 + t * 0.7, 9.2)),",
    "                fbm(p + 2.6 * q + vec2(8.3, 2.8 - t * 0.6)));",
    "  float f = fbm(p + 2.4 * r);",
    "",
    "  float band = clamp(f * 1.35 - 0.12, 0.0, 1.0);",
    "  vec3 col = palette(band);",
    "",
    "  // 辉光丝缕",
    "  float fil = smoothstep(0.58, 0.98, f) * 0.85;",
    "  col += fil * vec3(0.55, 0.6, 1.0) * 0.6;",
    "",
    "  // 暗角约束：中间亮、四周收",
    "  float vig = smoothstep(1.35, 0.25, length((uv - vec2(0.5, 0.46)) * asp));",
    "  col *= vig;",
    "",
    "  // 上下缘柔和压暗，保护文字可读性",
    "  col *= smoothstep(1.35, 0.6, abs(uv.y - 0.44) * 1.9);",
    "",
    "  gl_FragColor = vec4(col * uIntensity, 1.0);",
    "}"
  ].join("\n");

  function attach(canvas, opts) {
    if (!window.THREE) return null;
    opts = opts || {};

    var renderer;
    try {
      renderer = new THREE.WebGLRenderer({
        canvas: canvas,
        antialias: false,
        alpha: false,
        powerPreference: "high-performance"
      });
    } catch (e) {
      return null;
    }

    var dpr = Math.min(window.devicePixelRatio || 1, 1.6);
    renderer.setPixelRatio(dpr);

    var scene = new THREE.Scene();
    var camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);

    var uniforms = {
      uTime: { value: 0 },
      uRes: { value: new THREE.Vector2(1, 1) },
      uMouse: { value: new THREE.Vector2(0.5, 0.5) },
      uIntensity: { value: opts.intensity != null ? opts.intensity : 0.9 },
      uSeed: { value: opts.seed || Math.random() * 40 }
    };

    var material = new THREE.ShaderMaterial({
      vertexShader: VERT,
      fragmentShader: FRAG,
      uniforms: uniforms,
      depthWrite: false,
      depthTest: false
    });

    var quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), material);
    scene.add(quad);

    var mouseTarget = new THREE.Vector2(0.5, 0.5);
    function onPointer(ev) {
      var rect = canvas.getBoundingClientRect();
      if (rect.height < 1) return;
      mouseTarget.set(
        (ev.clientX - rect.left) / rect.width,
        1 - (ev.clientY - rect.top) / rect.height
      );
    }
    window.addEventListener("pointermove", onPointer, { passive: true });

    function resize() {
      var w = canvas.clientWidth, h = canvas.clientHeight;
      if (!w || !h) return;
      renderer.setSize(w, h, false);
      uniforms.uRes.value.set(w * dpr, h * dpr);
    }
    window.addEventListener("resize", resize);
    resize();

    var visible = true;
    var raf = 0;
    var start = performance.now();

    function frame(now) {
      raf = 0;
      var t = (now - start) / 1000;
      uniforms.uTime.value = REDUCED ? 0 : t;
      // 鼠标平滑跟随
      uniforms.uMouse.value.lerp(mouseTarget, 0.045);
      renderer.render(scene, camera);
      if (!REDUCED && visible) raf = requestAnimationFrame(frame);
    }

    var io = new IntersectionObserver(function (entries) {
      visible = entries[0].isIntersecting;
      if (visible && !raf && !REDUCED) raf = requestAnimationFrame(frame);
    }, { threshold: 0.02 });
    io.observe(canvas);

    if (REDUCED) {
      // 减少动态效果：渲染一帧静态极光
      resize();
      raf = requestAnimationFrame(function (now) {
        uniforms.uTime.value = 12.0;
        renderer.render(scene, camera);
      });
    } else {
      raf = requestAnimationFrame(frame);
    }

    return {
      resize: resize,
      dispose: function () {
        if (raf) cancelAnimationFrame(raf);
        io.disconnect();
        window.removeEventListener("pointermove", onPointer);
        window.removeEventListener("resize", resize);
        quad.geometry.dispose();
        material.dispose();
        renderer.dispose();
      }
    };
  }

  window.CodexNebula = { attach: attach };
})();
