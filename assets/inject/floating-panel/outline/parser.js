/* Answer Outline parser: heading candidates, levels, deduplication, and marks. */

  // Outline extraction is deliberately conservative: only visible, structured headings become targets.
  function outlineVisible(node) {
    if (!(node instanceof Element)) return false;
    const rect = node.getBoundingClientRect();
    return Boolean(rect.width > 8 && rect.height > 8);
  }

  function outlineMarkdownRoot(messageNode) {
    if (!(messageNode instanceof Element)) return null;
    const preferred = messageNode.querySelector(
      [
        "[class*='markdownContent']",
        "[class*='markdown-content']",
        ".markdown",
        ".prose",
        "article",
      ].join(",")
    );
    if (preferred && !preferred.closest(`[${ROOT_ATTR}="true"]`)) return preferred;
    return messageNode;
  }

  function outlineProtectedSurface(node) {
    if (!(node instanceof Element)) return true;
    return Boolean(node.closest([
      `[${ROOT_ATTR}="true"]`,
      "[contenteditable='true']",
      "textarea",
      "input",
      "form",
      ".ProseMirror",
    ].join(",")));
  }

  function outlineInCodeLike(node) {
    if (!(node instanceof Element)) return true;
    return Boolean(node.closest("pre, code, kbd, samp, [data-code-block], .cm-editor, .monaco-editor"));
  }

  function outlineInTableLike(node) {
    if (!(node instanceof Element)) return true;
    return Boolean(node.closest(OUTLINE_TABLE_SELECTOR));
  }

  function outlineHeadingLevelFromTag(tag) {
    const match = /^h([1-6])$/i.exec(tag || "");
    return match ? Number(match[1]) : 0;
  }

  function outlineIsMarkerOnlyTitle(text) {
    const value = normalizeText(text);
    if (!value) return true;
    if (/^[一二三四五六七八九十百零]+[、.．)]?$/.test(value)) return true;
    if (/^\d{1,2}[\.、．)]?$/.test(value)) return true;
    if (/^[（(]\d{1,2}[）)]$/.test(value)) return true;
    return /^#{1,6}$/.test(value);
  }

  function outlineIsNoiseTitle(text) {
    if (!text || outlineIsMarkerOnlyTitle(text)) return true;
    if (text.length < MIN_OUTLINE_TITLE_LEN || text.length > MAX_OUTLINE_TITLE_LEN) return true;
    if (text.length <= 4 && !/[0-9一二三四五六七八九十#：:]/.test(text) && !outlineHasChapterHeading(text)) return true;
    if (/^https?:\/\//i.test(text)) return true;
    if (/^[\w./~-]+\.(js|ts|json|md|py|sh|log|png|jpg)$/i.test(text)) return true;
    if (/^\$ |^>`|^```/.test(text)) return true;
    if (/^(复制|copy|edit|编辑|share|分享|continue|继续|retry|重试|项|实现|位置|范围|标题|跳转|折叠|刷新)$/i.test(text)) return true;
    if (/^[\d\s:./-]+$/.test(text)) return true;
    if (/^\/Users\/|^~\/|^\.\/|^\/Volumes\//.test(text)) return true;
    return /^(OK|PASS|FAIL|true|false|null)$/i.test(text);
  }

  function outlineHasChapterHeading(text) {
    const value = normalizeText(text);
    if (!value) return false;
    if (/^(摘要|简介|概述|概览|前言|背景|目标|现状|问题(?:分析)?|原因(?:分析)?|分析|方案|解决方案|步骤|实施步骤|实现|验证|验证结果|测试|测试结果|结果|结论|最终结论|总结|建议|后续建议|注意(?:事项)?|说明|补充说明|附录|下一步)(?:\s*[：:—-]\s*\S.*)?$/.test(value)) {
      return value.length <= 24;
    }
    return /^(abstract|introduction|overview|background|goals?|problems?|causes?|analysis|solutions?|steps?|implementation|verification|tests?|results?|conclusions?|summary|recommendations?|notes?|appendix|next steps?)(?:\s*[:：—-]\s*\S.*)?$/i.test(value)
      && value.length <= 32;
  }

  function outlineLooksStructuredHeading(text) {
    const value = normalizeText(text);
    if (!value || outlineIsMarkerOnlyTitle(value)) return false;
    if (/^#{1,6}\s+\S/.test(value)) return true;
    if (/^第[一二三四五六七八九十百零\d]+[章节部分步]/.test(value)) return true;
    if (/^[一二三四五六七八九十]+[、.．]\s*\S{2,}/.test(value)) return true;
    if (/^（?[0-9]{1,2}）\s*\S{2,}/.test(value) || /^\([0-9]{1,2}\)\s*\S{2,}/.test(value)) return true;
    if (/^\d{1,2}[\.、．\)]\s*\S{2,}/.test(value)) return true;
    return outlineHasChapterHeading(value);
  }

  function outlineScorePseudoHeading(text, levelHint) {
    let score = levelHint ? 20 : 0;
    if (!outlineLooksStructuredHeading(text) && !levelHint) return 0;
    if (/^#{1,6}\s+\S/.test(text)) score += 50;
    if (/^第[一二三四五六七八九十百零\d]+[章节部分步]/.test(text)) score += 30;
    if (/^[一二三四五六七八九十]+[、.．]\s*\S{2,}/.test(text)) score += 28;
    if (/^（?[0-9]{1,2}）\s*\S{2,}/.test(text) || /^\([0-9]{1,2}\)\s*\S{2,}/.test(text)) score += 24;
    if (/^\d{1,2}[\.、．\)]\s*\S{2,}/.test(text)) score += 26;
    if (/[：:]$/.test(text) && text.length <= 18 && text.length >= 4) score += 8;
    if (outlineHasChapterHeading(text)) score += 24;
    if (text.length >= 4 && text.length <= 20) score += 6;
    if (text.length >= 28) score -= 8;
    if (/[。！？]$/.test(text)) score -= 12;
    if (text.split(" ").length > 12) score -= 10;
    return score;
  }

  function outlineStripHeadingMarkers(text) {
    const stripped = normalizeText(text)
      .replace(/^#{1,6}\s+/, "")
      .replace(/^([（(]?\d{1,2}[）)]|[一二三四五六七八九十]{1,3}|\d{1,2})[\.、．\)]\s*/, "");
    return stripped && !outlineIsMarkerOnlyTitle(stripped) ? stripped : normalizeText(text);
  }

  function outlineDisplayHeadingTitle(text) {
    const value = normalizeText(text).replace(/^#{1,6}\s+/, "");
    return value.length <= MAX_OUTLINE_TITLE_LEN ? value : `${value.slice(0, MAX_OUTLINE_TITLE_LEN - 1)}…`;
  }

  function outlineTitlesEquivalent(left, right) {
    const a = normalizeText(left);
    const b = normalizeText(right);
    return Boolean(a && b && (a === b || outlineStripHeadingMarkers(a) === outlineStripHeadingMarkers(b)
      || outlineDisplayHeadingTitle(a) === outlineDisplayHeadingTitle(b)));
  }

  function outlineOwnsOwnLine(node, text) {
    if (!(node instanceof Element)) return false;
    const parent = node.parentElement;
    if (!parent) return true;
    const parentText = normalizeText(parent.innerText || parent.textContent || "");
    if (!parentText || parentText === text) return true;
    return parentText.startsWith(text) && parentText.length <= text.length + 4;
  }

  function outlineHeadingNumbering(text) {
    const value = normalizeText(text);
    const patterns = [
      [/^([一二三四五六七八九十]+[、.．])\s*(\S.*)$/, "han"],
      [/^(第[一二三四五六七八九十百零\d]+[章节部分步])\s*(\S.*)$/, "chapter"],
      [/^((?:（[0-9]{1,2}）|\([0-9]{1,2}\)))\s*(\S.*)$/, "arabic-parenthesized"],
    ];
    for (const [pattern, key] of patterns) {
      const match = value.match(pattern);
      if (match) return { prefix: match[1], title: match[2], pattern: key };
    }
    const arabic = value.match(/^(\d{1,2}(?:(?:[\.、．\)]\d{1,2})+)?[\.、．\)]?)\s+(\S.*)$/);
    if (!arabic) return { prefix: "", title: value, pattern: "" };
    const segments = arabic[1].match(/\d{1,2}/g)?.length || 1;
    const separators = arabic[1].match(/[\.、．\)]/g)?.join("") || ".";
    return {
      prefix: arabic[1],
      title: arabic[2],
      pattern: `arabic:${separators}:${segments}`,
    };
  }

  function outlineHeadingCandidate(node, kind) {
    if (!(node instanceof Element) || !outlineVisible(node) || outlineProtectedSurface(node) || outlineInCodeLike(node)) return null;
    if (node.closest(`[${ROOT_ATTR}="true"]`)) return null;

    const text = normalizeText(node.innerText || node.textContent || "");
    if (!text || text.length > MAX_OUTLINE_TITLE_LEN + 8) return null;
    const displayText = outlineDisplayHeadingTitle(text);
    if (outlineIsNoiseTitle(displayText) || outlineIsMarkerOnlyTitle(displayText)) return null;
    const numbering = outlineHeadingNumbering(displayText);

    if (kind === "semantic") {
      const tagLevel = outlineHeadingLevelFromTag(node.tagName);
      const ariaLevel = Number(node.getAttribute("aria-level") || 0);
      return {
        el: node,
        text: displayText,
        level: clamp(tagLevel || ariaLevel || 2, 1, 6),
        numberingPattern: numbering.pattern,
        numberPrefix: numbering.prefix,
        labelText: numbering.title,
        kind,
      };
    }

    if (outlineInTableLike(node)) return null;
    const childCount = node.children?.length || 0;
    if (childCount > 3 || node.querySelector("p,div,li,h1,h2,h3,h4,h5,h6,table,pre")) return null;

    if (node.matches("strong,b")) {
      if (!outlineOwnsOwnLine(node, text)) return null;
      const score = outlineScorePseudoHeading(text, 1) + 8;
      if (score < OUTLINE_PSEUDO_MIN_SCORE) return null;
      return {
        el: node,
        text: displayText,
        level: 3,
        numberingPattern: numbering.pattern,
        numberPrefix: numbering.prefix,
        labelText: numbering.title,
        kind,
      };
    }

    const rect = node.getBoundingClientRect();
    if (rect.height > 84 || !outlineLooksStructuredHeading(text)) return null;
    const score = outlineScorePseudoHeading(text, 0);
    if (score < OUTLINE_PSEUDO_MIN_SCORE) return null;
    return {
      el: node,
      text: displayText,
      level: numbering.pattern ? 2 : text.length <= 12 ? 2 : 3,
      numberingPattern: numbering.pattern,
      numberPrefix: numbering.prefix,
      labelText: numbering.title,
      kind,
    };
  }

  function outlineCollectSemanticHeadings(root) {
    if (!(root instanceof Element)) return [];
    const result = [];
    const nodes = root.querySelectorAll(OUTLINE_SEMANTIC_HEADING_SELECTOR);
    for (const node of nodes) {
      const item = outlineHeadingCandidate(node, "semantic");
      if (item) result.push(item);
    }
    return result;
  }

  function outlineCollectPseudoHeadings(root) {
    if (!(root instanceof Element)) return [];
    const result = [];
    const nodes = root.querySelectorAll(OUTLINE_PSEUDO_HEADING_SELECTOR);
    for (const node of nodes) {
      if (node.closest(OUTLINE_SEMANTIC_HEADING_SELECTOR)) continue;
      const item = outlineHeadingCandidate(node, "pseudo");
      if (item) result.push(item);
    }
    return result;
  }

  function outlineSortInDocumentOrder(items) {
    return items.slice().sort((left, right) => {
      if (left.el === right.el) return 0;
      const position = left.el.compareDocumentPosition(right.el);
      if (position & Node.DOCUMENT_POSITION_FOLLOWING) return -1;
      if (position & Node.DOCUMENT_POSITION_PRECEDING) return 1;
      return 0;
    });
  }

  function outlineCollectHeadingElements(root) {
    const semanticItems = outlineCollectSemanticHeadings(root);
    if (semanticItems.length >= MIN_OUTLINE_ITEMS) return outlineSortInDocumentOrder(semanticItems);
    return outlineSortInDocumentOrder([...semanticItems, ...outlineCollectPseudoHeadings(root)]);
  }

  function outlineDedupeItems(items) {
    const seen = new Set();
    const result = [];
    for (const item of items) {
      const key = `${item.level}|${item.text}`;
      if (seen.has(key)) continue;
      const previous = result.at(-1);
      if (previous && (previous.text === item.text || previous.el.contains(item.el) || item.el.contains(previous.el))) {
        continue;
      }
      seen.add(key);
      result.push(item);
      if (result.length >= MAX_OUTLINE_ITEMS) break;
    }
    return result;
  }

  function outlineNormalizeDisplayLevels(items) {
    if (!items.length) return items;
    const minimumLevel = Math.min(...items.map((item) => item.level));
    const numberedLevels = new Map();
    items.forEach((item) => {
      const baseLevel = item.level - minimumLevel;
      if (!item.numberingPattern) {
        item.displayLevel = baseLevel;
        return;
      }
      if (!numberedLevels.has(item.numberingPattern)) numberedLevels.set(item.numberingPattern, baseLevel);
      item.displayLevel = numberedLevels.get(item.numberingPattern);
    });
    return items;
  }

  function outlineMarkItems(items) {
    items.forEach((item, index) => {
      const id = `stepwise-outline-${hashText(`${index}:${item.text}`)}-${index + 1}`;
      item.id = id;
      item.el.setAttribute(MARK_ATTR, id);
    });
    return items;
  }

  function outlineClearMarks(root = document) {
    if (!root?.querySelectorAll) return;
    root.querySelectorAll(`[${MARK_ATTR}]`).forEach((node) => node.removeAttribute(MARK_ATTR));
    root.querySelectorAll(`.${HIGHLIGHT_CLASS}`).forEach((node) => node.classList.remove(HIGHLIGHT_CLASS));
  }
