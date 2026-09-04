/* Stepwise suggestions: payload parsing, normalization, labels, and deduplication. */

  function hideStepwisePayload(root) {
    if (!(root instanceof Element)) return;

    const blocks = Array.from(root.querySelectorAll("pre, code")).filter((node) => {
      if (!(node instanceof Element)) return false;
      return /"codex_stepwise"\s*:\s*true/.test(node.textContent || "");
    });

    for (const block of blocks) {
      const container = block.closest("[class*='_codeBlock_'], pre") || block;
      container.setAttribute(PAYLOAD_ATTR, "true");
    }
  }

  function clearStepwisePayloadMarks() {
    document.querySelectorAll(`[${PAYLOAD_ATTR}]`).forEach((node) => {
      node.removeAttribute(PAYLOAD_ATTR);
    });
  }

  function uniquePrompts(items) {
    const seen = new Set();
    const result = [];
    const maxItems = configuredMaxPromptItems();
    for (const item of Array.isArray(items) ? items : []) {
      const prompt = normalizeText(typeof item === "string" ? item : item.prompt);
      const dedupeKey = prompt.replace(/\s+/g, " ");
      if (!prompt || seen.has(dedupeKey)) continue;
      seen.add(dedupeKey);
      result.push({
        label: leadingPromptText(typeof item === "string" ? labelForPrompt(prompt) : item.label || labelForPrompt(prompt), 36),
        summary: leadingPromptText(
          typeof item === "string" ? summaryForPrompt(prompt) : item.summary || summaryForPrompt(prompt),
          MAX_PROMPT_SUMMARY_LENGTH,
        ),
        prompt,
      });
      if (result.length >= maxItems) break;
    }
    return result;
  }

  function normalizePromptState(items = state.prompts) {
    const normalized = uniquePrompts(items);
    state.prompts = normalized;
    state.promptPreviewIndex = normalized.length
      ? clamp(Number(state.promptPreviewIndex) || 0, 0, normalized.length - 1)
      : 0;
    return normalized;
  }

  function leadingPromptText(value, limit) {
    const characters = Array.from(normalizeText(value).replace(/\s+/g, " "));
    if (characters.length <= limit) return characters.join("");
    return `${characters.slice(0, Math.max(0, limit - 1)).join("").trimEnd()}…`;
  }

  function summaryForPrompt(prompt) {
    return leadingPromptText(prompt, MAX_PROMPT_SUMMARY_LENGTH);
  }

  function labelForPrompt(prompt) {
    const text = normalizeText(prompt);
    const rules = [
      [/diff|风险分级|改动.*总结/i, "查看 diff"],
      [/commit|提交/i, "整理 commit"],
      [/截图验证|遮挡|浮球|面板/i, "验证界面"],
      [/设置|配置|Bridge|API/i, "检查配置"],
      [/Codex\+\+|用户脚本|reload|生效/i, "检查脚本"],
      [/只读验证|确认.*生效|验证步骤/i, "验证生效"],
      [/错误|失败|最小复现|排查/i, "继续排查"],
      [/P0|P1|P2|执行顺序/i, "分级排序"],
      [/维护成本|长期稳定性|审查/i, "重新审查"],
      [/文件路径|当前状态|继续追踪/i, "列出路径"],
      [/下一步|改哪些文件/i, "继续下一步"],
      [/遗漏的风险|回滚方式/i, "风险回滚"],
    ];

    for (const [pattern, label] of rules) {
      if (pattern.test(text)) return label;
    }

    return text
      .replace(/^(帮我|请|把|给我|继续|检查|执行一次|基于刚才的)/, "")
      .replace(/[，。,.].*$/, "")
      .trim()
      .slice(0, 10) || "继续";
  }

  // Stepwise payload parsing accepts the backend's strict JSON contract and legacy embedded payload shapes.
  function parseStepwiseJson(text) {
    const blocks = Array.from(text.matchAll(/```(?:json)?\s*([\s\S]*?)```/gi))
      .map((match) => match[1])
      .filter((block) => /"codex_stepwise"\s*:\s*true/.test(block));

    for (const block of blocks.reverse()) {
      const parsed = parsePayloadCandidate(block);
      if (parsed) return parsed;
    }
    return parsePayloadCandidate(extractJsonObject(text));
  }

  function parsePayloadCandidate(value) {
    const text = normalizeText(value)
      .replace(/^```(?:json)?/i, "")
      .replace(/```$/i, "")
      .replace(/^json\s+/i, "")
      .trim();

    if (!/"codex_stepwise"\s*:\s*true/.test(text)) return null;

    try {
      const parsed = JSON.parse(text);
      return parsed && parsed.codex_stepwise === true ? parsed : null;
    } catch {
      return null;
    }
  }

  function extractJsonObject(text) {
    const source = String(text || "");
    const marker = source.search(/"codex_stepwise"\s*:\s*true/);
    if (marker < 0) return "";

    const start = source.lastIndexOf("{", marker);
    if (start < 0) return "";

    let depth = 0;
    let inString = false;
    let escaped = false;

    for (let index = start; index < source.length; index += 1) {
      const char = source[index];
      if (escaped) {
        escaped = false;
        continue;
      }
      if (char === "\\") {
        escaped = true;
        continue;
      }
      if (char === "\"") {
        inString = !inString;
        continue;
      }
      if (inString) continue;
      if (char === "{") depth += 1;
      if (char === "}") depth -= 1;
      if (depth === 0) return source.slice(start, index + 1);
    }

    return "";
  }

  function stripStepwisePayloadText(text) {
    const withoutFence = String(text || "").replace(/```(?:json)?\s*[\s\S]*?"codex_stepwise"\s*:\s*true[\s\S]*?```/gi, "");
    const payloadObject = extractJsonObject(withoutFence);
    return normalizeText(payloadObject ? withoutFence.replace(payloadObject, "") : withoutFence);
  }

  function payloadFromDom(root) {
    if (!(root instanceof Element)) return null;
    const blocks = Array.from(root.querySelectorAll("pre, code"))
      .filter((node) => /"codex_stepwise"\s*:\s*true/.test(node.textContent || ""));

    for (const block of blocks.reverse()) {
      const parsed = parsePayloadCandidate(block.textContent || "");
      if (parsed) return parsed;
    }

    return null;
  }

  function payloadItems(payload) {
    if (!payload) return [];
    if (Array.isArray(payload)) return payload;
    for (const key of ["items", "suggestions", "next_steps", "nextSteps", "actions", "prompts"]) {
      if (Array.isArray(payload[key])) return payload[key];
    }
    return [];
  }

  function payloadPrompts(payload) {
    const rawItems = payloadItems(payload);
    if (!rawItems.length) return [];
    const items = rawItems
      .slice(0, configuredMaxPromptItems())
      .map((item) => {
        const prompt = normalizeText(
          typeof item === "string"
            ? item
            : item?.prompt || item?.text || item?.action || item?.content || item?.message || "",
        );
        const label = leadingPromptText(
          typeof item === "string" ? "" : item?.label || item?.title || item?.name || "",
          36,
        );
        const summary = leadingPromptText(
          typeof item === "string" ? "" : item?.summary || item?.preview || item?.description || "",
          MAX_PROMPT_SUMMARY_LENGTH,
        );
        return prompt ? {
          label: label || labelForPrompt(prompt),
          summary: summary || summaryForPrompt(prompt),
          prompt,
        } : null;
      })
      .filter(Boolean);
    return uniquePrompts(items);
  }

  function extractStepwisePayload(message) {
    const text = elementText(message.node);
    const payload = payloadFromDom(message.node) || parseStepwiseJson(text);
    return {
      payload,
      prompts: payloadPrompts(payload),
      textWithoutPayload: stripStepwisePayloadText(text),
    };
  }
