/* ============================================================
   Codex++ 官网 · 交互主脚本
   - GSAP 入场与滚动编排（数字滚动、进度条）
   - 实拍大图预览与缩略图切换
   - 打字粒子特效（彩虹 / 烟花 / 星光）
   - 终端打字机、卡片 3D 倾斜、磁吸按钮
   所有效果遵循 prefers-reduced-motion，库缺失时自动降级。
   ============================================================ */

(function () {
  "use strict";

  var REDUCED = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var HAS_GSAP = typeof window.gsap !== "undefined";
  var HAS_ST = HAS_GSAP && typeof window.ScrollTrigger !== "undefined";

  if (HAS_ST) window.gsap.registerPlugin(window.ScrollTrigger);
  if (!HAS_GSAP || REDUCED) document.body.classList.add("no-anim");

  /* ---------------- 顶栏滚动态 ---------------- */

  var nav = document.getElementById("siteNav");
  function onScrollNav() {
    nav.classList.toggle("scrolled", window.scrollY > 30);
  }
  window.addEventListener("scroll", onScrollNav, { passive: true });
  onScrollNav();

  /* ---------------- 极光叠层 ---------------- */

  if (window.CodexNebula) {
    var heroCanvas = document.getElementById("nebula");
    var ctaCanvas = document.getElementById("nebula2");
    if (heroCanvas) window.CodexNebula.attach(heroCanvas, { intensity: 0.82, seed: 7.3 });
    if (ctaCanvas) window.CodexNebula.attach(ctaCanvas, { intensity: 0.55, seed: 21.7 });
  }

  /* ---------------- Hero 入场 ---------------- */

  if (HAS_GSAP && !REDUCED) {
    var tl = window.gsap.timeline({ defaults: { ease: "power4.out" } });
    tl.to(".hero-title .w", { y: 0, duration: 1.15, stagger: 0.09 }, 0.15)
      .to(".hero .eyebrow", { opacity: 1, y: 0, duration: 0.8 }, 0.35)
      .to(".hero-sub", { opacity: 1, y: 0, duration: 0.9 }, 0.7)
      .to(".hero-agents", { opacity: 1, y: 0, duration: 0.9 }, 0.78)
      .to(".hero-cta", { opacity: 1, y: 0, duration: 0.9 }, 0.85)
      .to(".hero-stats", { opacity: 1, y: 0, duration: 0.9 }, 1.0);

    // 背景照片缓慢推近
    window.gsap.to(".hero-photo", {
      scale: 1.02, duration: 18, ease: "sine.inOut", yoyo: true, repeat: -1
    });
  }

  /* ---------------- 数字滚动 ---------------- */

  function formatCount(el, v) {
    var fmt = el.getAttribute("data-format");
    var suffix = el.getAttribute("data-suffix") || "";
    if (fmt === "wan") return (v / 10000).toFixed(1) + "w";
    if (fmt === "m") return v.toFixed(0) + "M";
    return Math.round(v).toLocaleString("en-US") + suffix;
  }

  var counters = document.querySelectorAll("[data-count]");
  counters.forEach(function (el) {
    var target = parseFloat(el.getAttribute("data-count"));
    if (!HAS_GSAP || REDUCED) { el.textContent = formatCount(el, target); return; }
    var state = { v: 0 };
    window.gsap.to(state, {
      v: target,
      duration: 2.2,
      delay: 1.15,
      ease: "power3.out",
      onUpdate: function () { el.textContent = formatCount(el, state.v); }
    });
  });

  /* ---------------- 通用区块入场 ---------------- */

  if (HAS_ST && !REDUCED) {
    window.gsap.utils.toArray(".section-head, .fcard, .composer, .term, .bars, .dl-card, .cta-inner h2, .cta-sub, .cta-note").forEach(function (el) {
      window.gsap.from(el, {
        opacity: 0,
        y: 34,
        duration: 0.95,
        ease: "power3.out",
        scrollTrigger: { trigger: el, start: "top 86%", once: true }
      });
    });
  }

  /* ---------------- 点选实拍 ---------------- */

  (function screenshotGallery() {
    var showcase = document.getElementById("showcase");
    var track = document.getElementById("showcaseTrack");
    if (!showcase || !track) return;
    var shots = Array.from(track.querySelectorAll(".shot"));
    if (shots.length < 2 || shots.some(function (shot) {
      return !shot.querySelector("img") || !shot.querySelector("figcaption b");
    })) return;

    var controls = document.createElement("div");
    controls.className = "showcase-thumbs";
    controls.setAttribute("role", "group");
    controls.setAttribute("aria-label", document.documentElement.lang === "en" ? "Choose a screenshot" : "选择实拍截图");
    var buttons = shots.map(function (shot, index) {
      if (!shot.id) shot.id = "showcase-shot-" + (index + 1);
      var button = document.createElement("button");
      button.type = "button";
      button.className = "showcase-thumb";
      button.setAttribute("aria-controls", shot.id);
      button.setAttribute("aria-pressed", "false");
      var image = document.createElement("img");
      image.src = shot.querySelector("img").getAttribute("src");
      image.alt = "";
      image.loading = "lazy";
      var label = document.createElement("span");
      label.textContent = shot.querySelector("figcaption b").textContent;
      button.appendChild(image);
      button.appendChild(label);
      button.addEventListener("click", function () { selectShot(index); });
      controls.appendChild(button);
      return button;
    });

    function selectShot(index) {
      shots.forEach(function (shot, i) {
        shot.hidden = i !== index;
        buttons[i].setAttribute("aria-pressed", i === index ? "true" : "false");
      });
      shots[index].querySelector("img").loading = "eager";
    }

    // 控件完整创建并绑定后再收起其余截图，脚本不可用时仍可阅读全部图片。
    track.parentNode.insertBefore(controls, track.nextSibling);
    showcase.classList.add("is-gallery");
    selectShot(0);
  })();

  /* ---------------- 终端打字机 ---------------- */

  (function termTyper() {
    var body = document.querySelector("#term .term-body code");
    var out = document.getElementById("termOut");
    if (!body) return;
    var full = body.textContent;
    if (!HAS_ST || REDUCED) {
      if (out) out.classList.add("show");
      return;
    }
    body.textContent = "";
    var caret = document.createElement("span");
    caret.className = "caret";
    body.appendChild(caret);
    var started = false;

    function colorize(text) {
      return text
        .replace(/&/g, "&amp;").replace(/</g, "&lt;")
        .replace(/(#.*)$/gm, '<span class="t-cm">$1</span>')
        .replace(/(\[[0-9]+[KM]?\])/g, '<span class="t-hl">$1</span>');
    }

    window.ScrollTrigger.create({
      trigger: "#term",
      start: "top 74%",
      once: true,
      onEnter: function () {
        if (started) return;
        started = true;
        var i = 0;
        var timer = setInterval(function () {
          i += 1;
          var slice = full.slice(0, i);
          body.innerHTML = colorize(slice);
          body.appendChild(caret);
          if (i >= full.length) {
            clearInterval(timer);
            setTimeout(function () {
              caret.remove();
              if (out) out.classList.add("show");
              body.innerHTML = colorize(full);
            }, 420);
          }
        }, 26);
      }
    });
  })();

  /* ---------------- 窗口对比进度条 ---------------- */

  (function bars() {
    var rows = document.querySelectorAll(".bar-row");
    if (!rows.length) return;
    if (!HAS_ST || REDUCED) {
      rows.forEach(function (r) {
        r.querySelector(".bar-fill").style.width = r.getAttribute("data-pct") + "%";
      });
      return;
    }
    window.ScrollTrigger.create({
      trigger: ".bars",
      start: "top 82%",
      once: true,
      onEnter: function () {
        rows.forEach(function (r, idx) {
          window.gsap.to(r.querySelector(".bar-fill"), {
            width: r.getAttribute("data-pct") + "%",
            duration: 1.5,
            delay: idx * 0.18,
            ease: "power4.out"
          });
        });
      }
    });
  })();

  /* ---------------- 卡片 3D 倾斜 + 光斑跟随 ---------------- */

  (function tilt() {
    if (REDUCED || !window.matchMedia("(pointer: fine)").matches) return;
    document.querySelectorAll(".tilt").forEach(function (card) {
      var rect = null;
      var raf = 0;
      var rx = 0, ry = 0, tx = 0, ty = 0;

      function loop() {
        raf = 0;
        rx += (tx - rx) * 0.12;
        ry += (ty - ry) * 0.12;
        card.style.transform =
          "perspective(900px) rotateX(" + rx.toFixed(2) + "deg) rotateY(" + ry.toFixed(2) + "deg) translateZ(0)";
        if (Math.abs(tx - rx) > 0.01 || Math.abs(ty - ry) > 0.01) raf = requestAnimationFrame(loop);
      }
      function kick() { if (!raf) raf = requestAnimationFrame(loop); }

      card.addEventListener("pointerenter", function () {
        rect = card.getBoundingClientRect();
      });
      card.addEventListener("pointermove", function (ev) {
        if (!rect) return;
        var px = (ev.clientX - rect.left) / rect.width;
        var py = (ev.clientY - rect.top) / rect.height;
        ty = (px - 0.5) * 7;
        tx = (0.5 - py) * 7;
        card.style.setProperty("--mx", (px * 100).toFixed(1) + "%");
        card.style.setProperty("--my", (py * 100).toFixed(1) + "%");
        kick();
      });
      card.addEventListener("pointerleave", function () {
        tx = 0; ty = 0; rect = null;
        kick();
      });
    });
  })();

  /* ---------------- 磁吸按钮 ---------------- */

  (function magnetic() {
    if (REDUCED || !HAS_GSAP || !window.matchMedia("(pointer: fine)").matches) return;
    document.querySelectorAll(".magnetic").forEach(function (btn) {
      btn.addEventListener("pointermove", function (ev) {
        var rect = btn.getBoundingClientRect();
        window.gsap.to(btn, {
          x: (ev.clientX - rect.left - rect.width / 2) * 0.22,
          y: (ev.clientY - rect.top - rect.height / 2) * 0.32,
          duration: 0.5,
          ease: "power3.out"
        });
      });
      btn.addEventListener("pointerleave", function () {
        window.gsap.to(btn, { x: 0, y: 0, duration: 0.7, ease: "elastic.out(1, 0.45)" });
      });
    });
  })();

  /* ---------------- 供应商接入方式分段切换 ---------------- */

  (function modes() {
    var seg = document.querySelector(".seg");
    var pill = document.querySelector(".seg-pill");
    var panel = document.getElementById("modePanel");
    if (!seg || !panel) return;
    var title = document.getElementById("modeTitle");
    var desc = document.getElementById("modeDesc");
    var points = document.getElementById("modePoints");
    var route = document.getElementById("modeRoute");

    var EN = document.documentElement.lang === "en";
    var MODES = EN ? {
      official: {
        title: "Official sign-in",
        desc: "Use your ChatGPT / Codex account. Requests go through the official channel with the full official experience and plugin entry.",
        points: ["Sign in with your ChatGPT / Codex account", "Official quota and plugin marketplace", "Zero config, works out of the box"],
        nodes: ["You", "Codex", "OpenAI"]
      },
      hybrid: {
        title: "Official sign-in + API",
        desc: "Keep the official sign-in state and plugin entry while model requests always go through your configured compatible API — official quota is never consumed (there is no 'official first, API when exhausted' behavior).",
        points: ["Official UI on a custom API channel", "Model requests never touch official quota", "Chat and plugins unchanged"],
        nodes: ["You", "Codex", "Codex++", "Your API"]
      },
      api: {
        title: "API only",
        desc: "Bring your own Base URL / key, no official account needed. Chat Completions providers are converted locally to the Responses protocol Codex speaks.",
        points: ["Custom Base URL / key", "No official account required", "Chat Completions → Responses conversion"],
        nodes: ["You", "Codex", "Codex++", "Any relay"]
      },
      agg: {
        title: "Aggregate providers",
        desc: "Two flavors: failover switches members on errors with per-session / per-request / weighted rotation; model aggregation pins each model to a fixed member — members can even be signed-in official accounts.",
        points: ["Failover: auto-switch + rotation/weights", "Model aggregation: pin models to members", "Official accounts can be members too", "Model picker folds groups per provider"],
        nodes: ["You", "Codex", "Codex++", "Provider A / B / C"]
      }
    } : {
      official: {
        title: "官方登录",
        desc: "使用 ChatGPT / Codex 官方账号登录，模型请求走官方通道，保持官方应用的完整体验与插件入口。",
        points: ["ChatGPT / Codex 官方账号登录", "官方额度与官方插件市场", "零配置，开箱即用"],
        nodes: ["You", "Codex", "OpenAI"]
      },
      hybrid: {
        title: "官方登录 + API",
        desc: "保留官方登录状态与插件入口，模型请求始终走配置的兼容 API，不消耗官方额度（没有「先官方、耗尽再切 API」的行为）。",
        points: ["官方界面 + 自定义 API 通道", "模型请求不消耗官方额度", "聊天与插件体验不变"],
        nodes: ["You", "Codex", "Codex++", "你的 API"]
      },
      api: {
        title: "纯 API",
        desc: "使用自定义 Base URL / Key，无需官方账号。Chat Completions 供应商可通过本地代理转换为 Codex 使用的 Responses 协议。",
        points: ["自定义 Base URL / Key", "无需官方账号", "Chat Completions → Responses 协议转换"],
        nodes: ["You", "Codex", "Codex++", "任意中转"]
      },
      agg: {
        title: "聚合供应商",
        desc: "分「故障转移」与「模型聚合」两类：故障转移在多供应商间失败切换、按会话/请求/权重轮转；模型聚合按模型把请求固定到指定成员，成员也可以是已登录的官方账号。",
        points: ["故障转移：失败自动切换 + 轮转/权重", "模型聚合：按模型固定成员", "官方账号也可作为成员", "模型选择器按供应商分组折叠"],
        nodes: ["You", "Codex", "Codex++", "供应商 A / B / C"]
      }
    };

    function movePill(btn) {
      if (!pill) return;
      pill.style.width = btn.offsetWidth + "px";
      pill.style.transform = "translateX(" + (btn.offsetLeft - 5) + "px)";
    }

    function render(key, btn) {
      var m = MODES[key];
      if (!m) return;
      seg.querySelectorAll(".seg-btn").forEach(function (b) {
        var on = b === btn;
        b.classList.toggle("active", on);
        b.setAttribute("aria-selected", on ? "true" : "false");
      });
      movePill(btn);

      var apply = function () {
        title.textContent = m.title;
        desc.textContent = m.desc;
        points.innerHTML = m.points.map(function (p) { return "<li>" + p + "</li>"; }).join("");
        route.innerHTML = m.nodes.map(function (n, i) {
          var cls = "route-node" + (i === 0 ? " route-you" : i === m.nodes.length - 1 ? " route-hot" : "");
          var link = i < m.nodes.length - 1 ? '<span class="route-link"></span>' : "";
          return '<span class="' + cls + '">' + n + "</span>" + link;
        }).join("");
      };

      if (HAS_GSAP && !REDUCED) {
        window.gsap.to(panel, {
          opacity: 0, y: 10, duration: 0.18, ease: "power2.in",
          onComplete: function () {
            apply();
            window.gsap.to(panel, { opacity: 1, y: 0, duration: 0.32, ease: "power3.out" });
          }
        });
      } else {
        apply();
      }
    }

    var first = seg.querySelector(".seg-btn.active");
    requestAnimationFrame(function () { movePill(first); });
    window.addEventListener("resize", function () {
      movePill(seg.querySelector(".seg-btn.active"));
    });

    seg.addEventListener("click", function (ev) {
      var btn = ev.target.closest(".seg-btn");
      if (!btn || btn.classList.contains("active")) return;
      render(btn.getAttribute("data-mode"), btn);
    });
  })();

  /* ---------------- 最新动态（构建期注入的 CHANGELOG） ---------------- */

  (function releases() {
    var list = document.getElementById("relList");
    if (!list) return;
    var isEn = document.documentElement.lang === "en";
    fetch("data/releases.json", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : Promise.reject(new Error(r.status)); })
      .then(function (data) {
        var all = data.releases || [];
        if (all.length) {
          var latest = "v" + all[0].version;
          document.querySelectorAll("[data-site-version]").forEach(function (el) { el.textContent = latest; });
        }
        var rels = all.slice(0, 3);
        if (!rels.length) return;
        list.innerHTML = rels.map(function (rel, idx) {
          var items = rel.items.map(function (t) { return "<li>" + t + "</li>"; }).join("");
          return '<article class="rel-card' + (idx === 0 ? " latest" : "") + '">' +
            '<span class="rel-tag">' + (idx === 0 ? (isEn ? "● Latest · " : "● 最新 · ") : "") + "v" + rel.version + "</span>" +
            '<div class="rel-date">' + rel.date + "</div>" +
            "<ul>" + items + "</ul>" +
            '<a class="rel-more" href="https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v' + rel.version + '" target="_blank" rel="noopener">' + (isEn ? "View this release →" : "查看该版本 →") + "</a>" +
            "</article>";
        }).join("");
      })
      .catch(function () {
        list.innerHTML = isEn
          ? '<p class="rel-empty">Release notes could not be loaded. Check <a class="rel-more" href="https://github.com/BigPizzaV3/CodexPlusPlus/releases" target="_blank" rel="noopener">Releases</a> for the latest version.</p>'
          : '<p class="rel-empty">更新摘要暂时无法加载，到 <a class="rel-more" href="https://github.com/BigPizzaV3/CodexPlusPlus/releases" target="_blank" rel="noopener">Releases</a> 查看最新版本。</p>';
      });
  })();

  /* ---------------- 赞助商（Ad-List 实时清单 + 构建期快照兜底） ---------------- */

  (function sponsors() {
    var topBox = document.getElementById("sponsorTop");
    var grid = document.getElementById("sponsorGrid");
    var status = document.querySelector("[data-sponsor-status]");
    if (!grid) return;

    var REMOTE = [
      "https://ads.codexpp.cc/ads.json",
      "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
      "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json"
    ];

    function validUrl(value) {
      try {
        var u = new URL(String(value || "").trim());
        return u.protocol === "https:" || u.protocol === "http:" ? u.href : "";
      } catch (e) { return ""; }
    }

    function usable(ad, now) {
      if (!ad || ad.type !== "sponsor") return false;
      if (![ad.title, ad.description, ad.url].every(function (v) { return typeof v === "string" && v.trim(); })) return false;
      var exp = typeof ad.expires_at === "string" ? Date.parse(ad.expires_at) : NaN;
      return !(isFinite(exp) && exp < now);
    }

    function collect(payload, now) {
      var seen = {};
      var pick = function (items) {
        return (items || []).filter(function (ad) {
          if (!usable(ad, now)) return false;
          var u = validUrl(ad.url);
          if (!u) return false;
          var key = u.replace(/\/+$/, "").toLowerCase();
          if (seen[key]) return false;
          seen[key] = true;
          return true;
        }).map(function (ad) {
          return {
            title: ad.title.trim(),
            description: ad.description.trim(),
            url: validUrl(ad.url),
            image: validUrl(ad.image),
            highlights: Array.isArray(ad.highlights) ? ad.highlights.filter(function (h) { return typeof h === "string" && h.trim(); }).slice(0, 5) : []
          };
        });
      };
      var rawTop = payload && (payload.top_ad || payload.topAds || payload.topAd);
      var top = pick(Array.isArray(rawTop) ? rawTop : rawTop ? [rawTop] : []);
      var ads = pick(payload && payload.ads);
      return { top: top, ads: ads };
    }

    function cardEl(ad, featured) {
      var card = document.createElement("a");
      card.className = featured ? "sponsor-banner" : "sponsor-card";
      card.href = ad.url;
      card.target = "_blank";
      card.rel = "noreferrer";
      var mark = function () {
        var el = document.createElement("span");
        el.className = "sponsor-mark";
        el.textContent = ad.title.slice(0, 1);
        return el;
      };
      if (ad.image) {
        var img = document.createElement("img");
        img.src = ad.image;
        img.alt = "";
        img.loading = "lazy";
        img.addEventListener("error", function () { img.replaceWith(mark()); }, { once: true });
        card.appendChild(img);
      } else {
        card.appendChild(mark());
      }
      var copy = document.createElement("div");
      copy.className = "sponsor-copy";
      var h = document.createElement("h4");
      h.textContent = ad.title + " ↗";
      var p = document.createElement("p");
      p.textContent = ad.description;
      copy.appendChild(h);
      copy.appendChild(p);
      if (ad.highlights.length) {
        var ul = document.createElement("ul");
        ul.className = "sponsor-highlights";
        ad.highlights.forEach(function (text) {
          var li = document.createElement("li");
          li.textContent = text;
          ul.appendChild(li);
        });
        copy.appendChild(ul);
      }
      card.appendChild(copy);
      return card;
    }

    function render(data) {
      var c = collect(data, Date.now());
      topBox.innerHTML = "";
      grid.innerHTML = "";
      c.top.slice(0, 1).forEach(function (ad) { topBox.appendChild(cardEl(ad, true)); });
      c.ads.forEach(function (ad) { grid.appendChild(cardEl(ad, false)); });
      status.textContent = c.top.length + c.ads.length ? "" : "暂无有效赞助商。";
    }

    function tryFetch(urls, idx) {
      if (idx >= urls.length) return Promise.reject(new Error("all sources failed"));
      var controller = new AbortController();
      var timer = setTimeout(function () { controller.abort(); }, 8000);
      return fetch(urls[idx] + "?v=" + Date.now(), {
        cache: "no-store", credentials: "omit", signal: controller.signal
      }).then(function (r) {
        clearTimeout(timer);
        if (!r.ok) throw new Error(String(r.status));
        return r.json();
      }).catch(function (err) {
        clearTimeout(timer);
        return tryFetch(urls, idx + 1);
      });
    }

    // 实时清单优先；失败（离线/网络受限）回退构建期快照
    tryFetch(REMOTE, 0)
      .then(render)
      .catch(function () {
        return fetch("data/sponsors.json", { cache: "no-store" })
          .then(function (r) { return r.ok ? r.json() : Promise.reject(new Error(r.status)); })
          .then(render)
          .catch(function () { status.textContent = "赞助商列表暂时无法加载，请稍后重试或查看来源。"; });
      });
  })();

  /* ---------------- 打字粒子特效 ---------------- */

  (function typingFx() {
    var canvas = document.getElementById("fx");
    var input = document.getElementById("composerInput");
    if (!canvas || !input) return;
    var ctx = canvas.getContext("2d");
    var dpr = Math.min(window.devicePixelRatio || 1, 2);
    var particles = [];
    var hue = 0;
    var running = false;
    var raf = 0;

    function resize() {
      canvas.width = window.innerWidth * dpr;
      canvas.height = window.innerHeight * dpr;
      canvas.style.width = window.innerWidth + "px";
      canvas.style.height = window.innerHeight + "px";
    }
    window.addEventListener("resize", resize);
    resize();

    // 依据按键与上下文挑一种手感：enter=烟花，长句=星光，其余=彩虹粒子
    function spawn(x, y, kind, key) {
      if (REDUCED) return;
      var n = kind === "burst" ? 26 : kind === "spark" ? 10 : 7;
      for (var i = 0; i < n; i++) {
        var a, sp, life, size, col, grav;
        if (kind === "burst") {
          a = Math.random() * Math.PI * 2;
          sp = 1.8 + Math.random() * 4.4;
          life = 46 + Math.random() * 34;
          size = 1.6 + Math.random() * 2.4;
          col = "hsl(" + ((hue + Math.random() * 60 - 20 + 360) % 360) + " 95% 66%)";
          grav = 0.055;
        } else if (kind === "spark") {
          a = -Math.PI / 2 + (Math.random() - 0.5) * 1.7;
          sp = 0.6 + Math.random() * 1.8;
          life = 26 + Math.random() * 20;
          size = 1 + Math.random() * 1.8;
          col = Math.random() < 0.75 ? "#cfe4ff" : "#ffe9b8";
          grav = 0.012;
        } else {
          a = -Math.PI / 2 + (Math.random() - 0.5) * 2.4;
          sp = 0.8 + Math.random() * 2.6;
          life = 30 + Math.random() * 26;
          size = 1.4 + Math.random() * 2.2;
          col = "hsl(" + ((hue + Math.random() * 46) % 360) + " 92% 64%)";
          grav = 0.028;
        }
        particles.push({
          x: x, y: y,
          vx: Math.cos(a) * sp,
          vy: Math.sin(a) * sp,
          life: life, age: 0,
          size: size, color: col, grav: grav
        });
      }
      if (particles.length > 420) particles.splice(0, particles.length - 420);
      kick();
    }

    function caretPos() {
      var sel = window.getSelection();
      if (sel && sel.rangeCount) {
        var range = sel.getRangeAt(0).cloneRange();
        range.collapse(true);
        var rect = range.getClientRects()[0];
        if (rect) return { x: rect.left + rect.width, y: rect.top + rect.height * 0.5 };
      }
      var r = input.getBoundingClientRect();
      return { x: r.left + 40, y: r.top + 40 };
    }

    function kick() {
      if (!running && !REDUCED) {
        running = true;
        raf = requestAnimationFrame(frame);
      }
    }

    function frame() {
      raf = 0;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.globalCompositeOperation = "lighter";
      for (var i = particles.length - 1; i >= 0; i--) {
        var p = particles[i];
        p.age++;
        if (p.age >= p.life) { particles.splice(i, 1); continue; }
        p.vy += p.grav;
        p.vx *= 0.985;
        p.vy *= 0.99;
        p.x += p.vx;
        p.y += p.vy;
        var t = 1 - p.age / p.life;
        ctx.globalAlpha = t * t * 0.95;
        ctx.fillStyle = p.color;
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.size * (0.5 + t * 0.8), 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.restore();
      ctx.globalAlpha = 1;
      if (particles.length) {
        raf = requestAnimationFrame(frame);
      } else {
        running = false;
        ctx.clearRect(0, 0, canvas.width, canvas.height);
      }
    }

    input.addEventListener("keydown", function (ev) {
      if (ev.key === "Tab") { ev.preventDefault(); return; }
      if (ev.key === "Enter") { ev.preventDefault(); document.execCommand("insertLineBreak"); }
      if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
      hue = (hue + 11) % 360;
      // 等浏览器把字符插入后再取光标位置
      requestAnimationFrame(function () {
        var pos = caretPos();
        var kind = ev.key === "Enter" ? "burst"
          : (input.textContent.length % 24 > 18 ? "spark" : "rainbow");
        spawn(pos.x, pos.y, kind, ev.key);
      });
    });

    input.addEventListener("paste", function (ev) {
      ev.preventDefault();
      var text = (ev.clipboardData || window.clipboardData).getData("text/plain");
      document.execCommand("insertText", false, text);
      hue = (hue + 47) % 360;
      requestAnimationFrame(function () {
        var pos = caretPos();
        spawn(pos.x, pos.y, "burst");
      });
    });

    // 初次进入视口，欢迎烟花
    var welcomed = false;
    var io = new IntersectionObserver(function (entries) {
      if (entries[0].isIntersecting && !welcomed && !REDUCED) {
        welcomed = true;
        var r = input.getBoundingClientRect();
        hue = 205;
        setTimeout(function () { spawn(r.left + 70, r.top + 46, "burst"); }, 350);
        setTimeout(function () { spawn(r.left + r.width * 0.5, r.top + 60, "rainbow"); }, 650);
      }
    }, { threshold: 0.4 });
    io.observe(input);
  })();

  /* ---------------- 平滑锚点（顶栏偏移补偿） ---------------- */

  document.querySelectorAll("[data-nav]").forEach(function (a) {
    a.addEventListener("click", function (ev) {
      var id = a.getAttribute("href");
      if (!id || id.charAt(0) !== "#") return;
      var target = document.querySelector(id);
      if (!target) return;
      ev.preventDefault();
      var y = id === "#top" ? 0 : target.getBoundingClientRect().top + window.scrollY - 66;
      window.scrollTo({ top: y, behavior: REDUCED ? "auto" : "smooth" });
    });
  });

})();
