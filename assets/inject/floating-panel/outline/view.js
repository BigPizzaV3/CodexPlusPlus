/* Answer Outline view: markup, events, and panel content scroll state. */

  function alignOutlineNestedText() {
    const rows = Array.from(state.panel?.querySelectorAll?.(".csw-outline-row[data-outline-id]") || []);
    const numberedAncestors = [];
    rows.forEach((row) => {
      const level = Math.max(0, Number(row.dataset.level) || 0);
      while (numberedAncestors.length && numberedAncestors.at(-1).level >= level) numberedAncestors.pop();

      const ancestor = numberedAncestors.at(-1);
      if (row.dataset.numbered === "true") {
        row.style.removeProperty("--csw-outline-hanging-indent");
        const prefix = row.querySelector(".csw-outline-prefix");
        numberedAncestors.push({
          level,
          width: Math.max(
            0,
            (prefix?.getBoundingClientRect().width || 0) + 8 - OUTLINE_INDENT_STEP,
          ),
        });
        return;
      }

      row.style.setProperty(
        "--csw-outline-hanging-indent",
        ancestor ? `${ancestor.width}px` : "0px",
      );
    });
  }

  function outlineHtml() {
    if (state.outlineStatus === "pending") {
      return `<div class="csw-progress" aria-label="正在整理大纲">
        <span class="csw-progress-ring" aria-hidden="true"></span>
        <span class="csw-progress-copy">
          <span class="csw-progress-title">正在整理大纲</span>
        </span>
      </div>`;
    }
    if (state.outlineStatus === "error") {
      return `<div class="csw-empty" data-kind="outline">
        <div class="csw-empty-title">${escapeHtml(outlineErrorTitle())}</div>
      </div>`;
    }
    if (!state.outlineItems.length) {
      return `<div class="csw-empty" data-kind="outline">
        <div class="csw-empty-title">暂无大纲</div>
      </div>`;
    }
    return `<div class="csw-outline-view">
      <div class="csw-outline-list" role="list">${state.outlineItems.map((item) => {
      const displayLevel = item.displayLevel ?? 0;
      const numberPrefix = item.numberPrefix || "";
      const labelText = item.labelText || item.text;
      return `
        <button class="csw-outline-row" type="button" role="listitem" data-outline-id="${escapeAttr(item.id)}" data-level="${displayLevel}" data-numbered="${numberPrefix ? "true" : "false"}" aria-label="${escapeAttr(item.text)}" style="--csw-outline-indent:${Math.min(3, Math.max(0, displayLevel)) * OUTLINE_INDENT_STEP}px">
          <span class="csw-outline-heading-marker" aria-hidden="true"></span>
          <span class="csw-outline-prefix" aria-hidden="true">${escapeHtml(numberPrefix)}</span>
          <span class="csw-outline-label">${escapeHtml(labelText)}</span>
        </button>
      `;
      }).join("")}
      </div>
      <div class="csw-outline-toolbar" role="toolbar" aria-label="本轮导航">
        <button class="csw-outline-nav-button" type="button" data-outline-anchor="start" title="本轮开头" aria-label="定位到本轮开头">${iconSvg("turn-start")}</button>
        <button class="csw-outline-nav-button" type="button" data-outline-anchor="end" title="本轮结尾" aria-label="定位到本轮结尾">${iconSvg("turn-end")}</button>
      </div>
    </div>`;
  }

  function attachOutlineEvents() {
    state.panel.querySelectorAll("[data-outline-id]").forEach((button) => {
      button.addEventListener("click", () => {
        if (!outlineJumpTo(button.dataset.outlineId)) {
          state.outlineStatus = "error";
          state.outlineError = "找不到对应的小节，刷新后再试。";
          renderFloat({ preserveMorph: true });
        }
      });
    });
    state.panel.querySelectorAll("[data-outline-anchor]").forEach((button) => {
      button.addEventListener("click", () => {
        if (!outlineJumpToAnchor(button.dataset.outlineAnchor)) {
          state.outlineStatus = "error";
          state.outlineError = "找不到当前回答位置，刷新后再试。";
          renderFloat({ preserveMorph: true });
        }
      });
    });
  }
