  /**
   * 内置拓展「自定义 prompt 参数」。
   *
   * Codex 应用会把 <CODEX_HOME>/prompts/*.md 做成 `/prompts:<名字>` 斜杠命令，但发送时只是
   * 「正文 + 空格 + 参数」，不认 CLI 的占位符；prompt 清单也只在页面加载时读一次。
   * 这里在回车发送 `/prompts:<名字> 参数` 时按 CLI 规则展开，再交给 Codex 发出：
   *
   *   $1 … $9      第 N 个参数（按空白拆分，引号包住的算一个）；没给的换成空
   *   $ARGUMENTS   全部参数
   *   $名字        具名参数（大写字母开头），用 名字=值 传；缺了提示、不发送
   *   $$           字面的 $
   *
   * 没有占位符的 prompt 与原生一致（正文 + 参数）。只处理带参数的输入：只输「/prompts:名字」时
   * 原生会把参数提示填进输入框，不去抢。清单由后端 /custom-prompts/list 读，新加的 prompt 不用重载页面。
   */
  const codexPlusPromptArgsPattern = /^\/prompts:([^\s/]+)\s+([\s\S]*\S)\s*$/;
  const codexPlusPromptArgsNamedPattern = /^[A-Z][A-Z0-9_]*/;
  const codexPlusPromptArgsCacheMs = 3000;
  const codexPlusPromptArgsCache = { prompts: new Map(), at: 0, promise: null };

  function codexPlusPromptArgsEnabled() {
    return codexPlusBackendSettingsLoaded === true && codexPlusSettings().promptArgs === true;
  }

  /** 按空白拆参数；"..." 与 '...' 包住的算一个，双引号里 \" 表示引号本身。 */
  function codexPlusPromptArgsSplit(text) {
    const args = [];
    let current = "";
    let quote = "";
    let started = false;
    const source = String(text || "");
    for (let index = 0; index < source.length; index += 1) {
      const char = source[index];
      if (quote) {
        if (char === quote) quote = "";
        else if (quote === "\"" && char === "\\" && (source[index + 1] === "\"" || source[index + 1] === "\\")) current += source[++index];
        else current += char;
      } else if (char === "\"" || char === "'") {
        quote = char;
        started = true;
      } else if (/\s/.test(char)) {
        if (started) args.push(current);
        current = "";
        started = false;
      } else {
        current += char;
        started = true;
      }
    }
    if (started) args.push(current);
    return args;
  }

  /** 正文里的占位符：positional（$1-$9 / $ARGUMENTS）与具名名字列表（按出现顺序去重）。 */
  function codexPlusPromptArgsPlaceholders(content) {
    const text = String(content || "");
    const named = [];
    let positional = false;
    for (let index = 0; index < text.length; index += 1) {
      if (text[index] !== "$") continue;
      const next = text[index + 1];
      if (next === "$") {
        index += 1;
        continue;
      }
      if (next >= "1" && next <= "9") {
        positional = true;
        continue;
      }
      const name = codexPlusPromptArgsNamedPattern.exec(text.slice(index + 1))?.[0];
      if (!name) continue;
      if (name === "ARGUMENTS") positional = true;
      else if (!named.includes(name)) named.push(name);
      index += name.length;
    }
    return { positional, named };
  }

  /** 单遍替换：values 给具名值；args 给位置参数（null 表示这一类不替换，原样保留）。 */
  function codexPlusPromptArgsSubstitute(content, { args = null, values = null } = {}) {
    const text = String(content || "");
    let out = "";
    for (let index = 0; index < text.length; index += 1) {
      const char = text[index];
      if (char !== "$") {
        out += char;
        continue;
      }
      const next = text[index + 1];
      if (next === "$") {
        out += "$";
        index += 1;
        continue;
      }
      if (args && next >= "1" && next <= "9") {
        out += args[Number(next) - 1] ?? "";
        index += 1;
        continue;
      }
      const name = codexPlusPromptArgsNamedPattern.exec(text.slice(index + 1))?.[0];
      if (name === "ARGUMENTS" && args) {
        out += args.join(" ");
        index += name.length;
      } else if (name && values && Object.prototype.hasOwnProperty.call(values, name)) {
        out += values[name];
        index += name.length;
      } else {
        out += char;
      }
    }
    return out;
  }

  /**
   * 展开一条 prompt。返回 { ok: true, text } / { ok: false, error } / { ok: true, text, hint }。
   *
   * 具名模式要「正文有具名占位符、且参数里至少有一个 已知名字=值」才进：正文里写了 $HOME、$PATH 这类
   * shell 变量又没按 名字=值 传参时，照位置参数 / 原生方式处理，只提示一句，不拦着发送。
   */
  function codexPlusPromptArgsExpand(prompt, argText) {
    const content = String(prompt?.content || "");
    const args = codexPlusPromptArgsSplit(argText);
    const { positional, named } = codexPlusPromptArgsPlaceholders(content);
    const pairs = args.map((arg) => {
      const at = arg.indexOf("=");
      return at > 0 ? { key: arg.slice(0, at), value: arg.slice(at + 1) } : null;
    });
    const namedMode = named.length > 0 && pairs.some((pair) => pair && named.includes(pair.key));
    if (namedMode) {
      const bad = args.filter((_, index) => !pairs[index]);
      if (bad.length) {
        return { ok: false, error: `/prompts:${prompt.id} 的参数要写成 名字=值（值里有空格就加引号），这些没写对：${bad.join("、")}` };
      }
      const values = {};
      for (const pair of pairs) values[pair.key] = pair.value;
      const missing = named.filter((name) => !Object.prototype.hasOwnProperty.call(values, name));
      if (missing.length) {
        return { ok: false, error: `/prompts:${prompt.id} 缺少参数：${missing.join("、")}（写成 ${missing[0]}=值）` };
      }
      return { ok: true, text: codexPlusPromptArgsSubstitute(content, { values }).trim() };
    }
    const hint = named.length
      ? `/prompts:${prompt.id} 里有 ${named.map((name) => `$${name}`).join("、")}，要替换请写成 ${named[0]}=值`
      : "";
    if (positional) {
      return { ok: true, text: codexPlusPromptArgsSubstitute(content, { args }).trim(), hint };
    }
    // 没有可替换的占位符：和原生一样拼在正文后面。
    return { ok: true, text: `${content} ${String(argText || "").trim()}`.trim(), hint };
  }

  function codexPlusPromptArgsRefresh(force = false) {
    if (codexPlusPromptArgsCache.promise) return codexPlusPromptArgsCache.promise;
    if (!force && Date.now() - codexPlusPromptArgsCache.at < codexPlusPromptArgsCacheMs) {
      return Promise.resolve(codexPlusPromptArgsCache.prompts);
    }
    codexPlusPromptArgsCache.promise = postJson("/custom-prompts/list", {})
      .then((result) => {
        const prompts = new Map();
        for (const prompt of Array.isArray(result?.prompts) ? result.prompts : []) {
          if (prompt && typeof prompt.id === "string" && typeof prompt.content === "string") prompts.set(prompt.id, prompt);
        }
        codexPlusPromptArgsCache.prompts = prompts;
        codexPlusPromptArgsCache.at = Date.now();
        return prompts;
      })
      .catch(() => codexPlusPromptArgsCache.prompts)
      .finally(() => {
        codexPlusPromptArgsCache.promise = null;
      });
    return codexPlusPromptArgsCache.promise;
  }

  /** 换好文字后让 Codex 自己发：点发送按钮；找不到就留在输入框里让用户再按一次回车。 */
  function codexPlusPromptArgsSubmit(editor) {
    const button = typeof codexPlusDictationSendButton === "function" ? codexPlusDictationSendButton(editor) : null;
    if (button) {
      button.click();
      return true;
    }
    showToast("已按 prompt 展开参数，按回车发送");
    return false;
  }

  function codexPlusPromptArgsKeyDown(event) {
    if (event.key !== "Enter" || event.isComposing || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey) return;
    if (!codexPlusPromptArgsEnabled()) return;
    const editor = codexPlusComposerEditorFrom(event.target);
    if (!editor) return;
    const match = codexPlusPromptArgsPattern.exec((editor.innerText || "").trim());
    if (!match) return;
    const prompt = codexPlusPromptArgsCache.prompts.get(match[1]);
    // 清单里没有：可能还没读到或文件刚删，交给原生。
    if (!prompt) return;
    const result = codexPlusPromptArgsExpand(prompt, match[2]);
    event.preventDefault();
    event.stopImmediatePropagation();
    if (!result.ok) {
      showToast(result.error, { type: "error" });
      return;
    }
    if (result.hint) showToast(result.hint, { type: "warn" });
    codexPlusClearComposer(editor);
    if (result.text) document.execCommand("insertText", false, result.text);
    // 等编辑器把新内容同步进自己的状态再点发送。
    setTimeout(() => codexPlusPromptArgsSubmit(editor), 30);
  }

  function codexPlusPromptArgsInput(event) {
    if (!codexPlusPromptArgsEnabled()) return;
    const editor = codexPlusComposerEditorFrom(event.target);
    if (editor && /^\s*\/prompts:/.test(editor.innerText || "")) void codexPlusPromptArgsRefresh();
  }

  function codexPlusPromptArgsExtension(ctx) {
    ctx.listen(document, "keydown", codexPlusPromptArgsKeyDown, { capture: true });
    ctx.listen(document, "input", codexPlusPromptArgsInput, { capture: true });
    if (codexPlusPromptArgsEnabled()) void codexPlusPromptArgsRefresh(true);
  }
