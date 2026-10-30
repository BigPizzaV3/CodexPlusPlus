/* ============================================================
   Codex++ 官网 · 交互脚本（首页与使用教程共用）
   - 明暗主题切换（记在 localStorage，读写失败时照常跟随系统）
   - 首屏产品导览页签；「在线体验」页签第一次打开时才加载 iframe
   - 滚动入场、截图大图预览
   - 最新动态（构建期生成的 releases.json）与赞助商（Ad-List 实时清单 + 快照兜底）
   没有 JS 时页面内容照常可读：入场动画只在 html.js 下生效。
   ============================================================ */

(function () {
  "use strict";

  var root = document.documentElement;
  var EN = root.lang === "en";
  var REDUCED = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var THEME_KEY = "codexpp-theme";

  /* ---------------- 主题 ---------------- */

  function systemDark() { return window.matchMedia("(prefers-color-scheme: dark)").matches; }
  function currentDark() {
    var t = root.getAttribute("data-theme");
    return t ? t === "dark" : systemDark();
  }
  document.querySelectorAll("[data-theme-toggle]").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var next = currentDark() ? "light" : "dark";
      root.setAttribute("data-theme", next);
      try { localStorage.setItem(THEME_KEY, next); } catch (e) {}
    });
  });

  /* ---------------- 顶栏滚动态 ---------------- */

  var nav = document.getElementById("siteNav");
  if (nav && !nav.classList.contains("solid")) {
    var onScroll = function () { nav.classList.toggle("scrolled", window.scrollY > 8); };
    window.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
  }

  /* ---------------- 首屏产品导览 ---------------- */

  (function tour() {
    var tabs = Array.prototype.slice.call(document.querySelectorAll(".tour-tab"));
    if (!tabs.length) return;
    var cap = document.getElementById("tourCap");
    var bar = document.getElementById("tourTitle");

    function select(tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.setAttribute("aria-selected", on ? "true" : "false");
        t.tabIndex = on ? 0 : -1;
        var panel = document.getElementById(t.getAttribute("aria-controls"));
        if (!panel) return;
        panel.classList.toggle("active", on);
        if (on) {
          var frame = panel.querySelector("iframe[data-src]");
          if (frame) { frame.src = frame.getAttribute("data-src"); frame.removeAttribute("data-src"); }
        }
      });
      if (cap) cap.textContent = tab.getAttribute("data-cap") || "";
      if (bar) bar.textContent = tab.getAttribute("data-title") || "";
      if (focus) tab.focus();
    }

    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () { select(tab, false); });
      tab.addEventListener("keydown", function (ev) {
        var step = ev.key === "ArrowRight" ? 1 : ev.key === "ArrowLeft" ? -1 : 0;
        if (!step) return;
        ev.preventDefault();
        select(tabs[(i + step + tabs.length) % tabs.length], true);
      });
    });
    select(tabs.filter(function (t) { return t.getAttribute("aria-selected") === "true"; })[0] || tabs[0], false);
  })();

  /* ---------------- 滚动入场 ---------------- */

  (function reveal() {
    var items = document.querySelectorAll(".reveal");
    if (!items.length) return;
    if (REDUCED || !("IntersectionObserver" in window)) {
      items.forEach(function (el) { el.classList.add("in"); });
      return;
    }
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("in");
        io.unobserve(entry.target);
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 });
    items.forEach(function (el) { io.observe(el); });
  })();

  /* ---------------- 截图大图预览 ---------------- */

  (function lightbox() {
    var box = document.getElementById("lightbox");
    if (!box || typeof box.showModal !== "function") return;
    var img = box.querySelector("img");
    var text = box.querySelector("p");
    document.addEventListener("click", function (ev) {
      var trigger = ev.target.closest("[data-zoom]");
      if (!trigger) return;
      var source = trigger.tagName === "IMG" ? trigger : trigger.querySelector("img");
      if (!source) return;
      ev.preventDefault();
      img.src = source.currentSrc || source.src;
      img.alt = source.alt;
      text.textContent = source.alt;
      box.showModal();
    });
    box.addEventListener("click", function (ev) {
      if (ev.target === box || ev.target.closest(".lightbox-close")) box.close();
    });
  })();

  /* ---------------- 最新动态（构建期注入的 CHANGELOG） ---------------- */

  (function releases() {
    var list = document.getElementById("relList");
    fetch("data/releases.json", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : Promise.reject(new Error(r.status)); })
      .then(function (data) {
        var all = data.releases || [];
        if (all.length) {
          var latest = "v" + all[0].version;
          document.querySelectorAll("[data-site-version]").forEach(function (el) { el.textContent = latest; });
        }
        if (!list) return;
        var rels = all.slice(0, 3);
        if (!rels.length) return;
        list.innerHTML = rels.map(function (rel, idx) {
          var items = rel.items.slice(0, 4).map(function (t) { return "<li>" + t + "</li>"; }).join("");
          return '<article class="rel-card' + (idx === 0 ? " latest" : "") + '">' +
            '<div class="rel-head"><span class="rel-tag">v' + rel.version + "</span>" +
            '<span class="rel-date">' + rel.date + "</span>" +
            (idx === 0 ? '<span class="rel-new">' + (EN ? "Latest" : "最新") + "</span>" : "") + "</div>" +
            "<ul>" + items + "</ul>" +
            '<a class="rel-more" href="https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v' + rel.version + '" target="_blank" rel="noopener">' + (EN ? "Release notes →" : "查看该版本 →") + "</a>" +
            "</article>";
        }).join("");
      })
      .catch(function () {
        if (!list) return;
        list.innerHTML = EN
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
      status.textContent = c.top.length + c.ads.length ? "" : (EN ? "No active sponsors." : "暂无有效赞助商。");
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
      }).catch(function () {
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
          .catch(function () {
            status.textContent = EN ? "The sponsor list could not be loaded. Try again later or check the source." : "赞助商列表暂时无法加载，请稍后重试或查看来源。";
          });
      });
  })();
})();
