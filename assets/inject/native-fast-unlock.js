(() => {
  const root = typeof window === "object" ? window : {};
  const id = "[A-Za-z_$][\\w$]*";
  const chatgpt = "`chatgpt`";
  const token = "`personalAccessToken`";
  const apiKey = "`apiKey`";
  const local = "`local`";
  const critical = "`critical`";
  // 新版 Codex 会往同一门控里追加官方登录方式（如 amazonBedrock），只接受纯字面量。
  const literal = "`[A-Za-z]\\w*`";

  const runtime = {
    update(settings) {
      root.__codexPlusSettings = settings && typeof settings === "object" ? settings : null;
      return runtime.status();
    },
    status() {
      const settings = root.__codexPlusSettings;
      const enhancementsEnabled = settings?.enhancementsEnabled === true;
      const optedIn = settings?.codexAppNativeFastUnlock === true;
      return {
        enabled: enhancementsEnabled && optedIn,
        enhancementsEnabled,
        optedIn,
        localApiConfigured: settings?.nativeFastLocalApiConfigured === true,
        settingsLoaded: !!settings && typeof settings === "object",
      };
    },
    allow(hostId, authMethod) {
      const status = runtime.status();
      // 自定义供应商直接携带凭据时原生账号可能为空；只能使用后端对生效配置的布尔投影，
      // 不能把所有 null 身份当作 API key，更不能借本机配置放行远端或其它认证方式。
      return hostId === "local" && status.enabled
        && (authMethod === "apikey" || authMethod === "apiKey"
          || authMethod == null && status.localApiConfigured);
    },
  };

  function allow(host, auth) {
    return `window.__codexPlusNativeFastUnlock?.runtime?.allow?.(${host},${auth})===true`;
  }

  // 只识别已核对的四条完整调用结构；压缩函数名和局部变量名都可变化。
  // 不修改 requirements、模型 serviceTiers 或 Standard 的解析规则。
  const gates = [
    {
      kind: "ui-access",
      pattern: [
        `function ${id}\\((?<argument>${id})\\)\\{let ${id}=\\(0,${id}\\.c\\)\\(\\d+\\),(?<fallback>${id})=${id}\\(${id}\\),`,
        `(?<host>${id})=\\k<argument>\\?\\.hostId\\?\\?\\k<fallback>,(?<identity>${id})=${id}\\(\\k<host>\\),`,
        `(?<variable>${id})=(?<gate>\\k<identity>\\?\\.authMethod===${chatgpt}\\|\\|\\k<identity>\\?\\.authMethod===${token}(?:\\|\\|\\k<identity>\\?\\.authMethod===${literal})*),`,
        `(?<method>${id})=\\k<identity>\\?\\.authMethod\\?\\?null,[^{}]{0,300}\\{authMethod:\\k<method>,hostId:\\k<host>\\}`,
        `[^{}]{0,300}\\{data:(?<data>${id}),isPending:(?<pending>${id})\\}=${id}\\(${id},${id}\\),`,
        `(?<loading>${id})=!!\\k<identity>\\?\\.isLoading\\|\\|\\k<variable>&&\\k<pending>,${id};`,
        `[^{}]{0,300}\\k<variable>&&!\\k<loading>&&\\k<data>!=null\\?${id}\\(\\k<data>\\):null`,
        `[^{}]{0,300}\\{serviceTierAccess:${id},isLoading:\\k<loading>\\}`,
      ].join(""),
      replacement: (groups) => `(${groups.gate}||${allow(groups.host, `${groups.identity}?.authMethod`)})`,
    },
    {
      kind: "request-access",
      pattern: [
        `async function ${id}\\((?<scope>${id}),(?<host>${id})\\)\\{let (?<auth>${id})=await ${id}\\(\\k<scope>,\\k<host>\\);`,
        `if\\((?<gate>\\k<auth>!==${chatgpt}&&\\k<auth>!==${token}(?:&&\\k<auth>!==${literal})*)\\)return null;`,
        `let (?<requirements>${id})=await ${id}\\(\\k<scope>,\\k<host>,\\{priority:${critical}\\}\\);`,
        `return \\k<scope>\\.query\\.setData\\(${id},\\{authMethod:\\k<auth>,hostId:\\k<host>\\},\\k<requirements>\\),${id}\\(\\k<requirements>\\)\\}`,
      ].join(""),
      replacement: (groups) => `${groups.gate}&&!(${allow(groups.host, groups.auth)})`,
    },
    {
      kind: "queue-default-access",
      variants: [
        {
          pattern: [
            `function ${id}\\((?<manager>${id}),\\{runtime:(?<runtime>${id}),storage:(?<storage>${id})\\},(?<before>${id}),${id}\\)\\{let (?<pending>${id})=null;`,
            `return\\{async readServiceTier\\((?<model>${id})\\)\\{for\\(;\\k<pending>!=null;\\)await \\k<pending>\\.catch\\(\\(\\)=>void 0\\);`,
            `return \\k<before>\\(\\),${id}\\(\\k<manager>\\.requestClient,\\k<manager>\\.logger,\\k<model>,async\\(\\)=>`,
            `\\k<manager>\\.getHostId\\(\\)===${local}&&await \\k<storage>\\.readGlobalState\\(\`use-copilot-auth-if-available\`\\)===!0&&await \\k<runtime>\\.isCopilotApiAvailable\\(\\)\\?!1:`,
            `(?<gate>(?<account>\\(await \\k<manager>\\.getAccount\\(\\{priority:${critical}\\}\\)\\)\\.account\\?\\.type)===${chatgpt})\\)\\},async writeServiceTier\\(`,
          ].join(""),
          // includes 只消费一次 account/read；空账号也必须经过同一份本机 API 策略。
          // 外层 Copilot 分支原样保留，不能用本机自定义供应商冒充所选 Copilot。
          replacement: (groups) => `[${chatgpt},...(${allow(`${groups.manager}.getHostId()`, apiKey)}?[${apiKey}]:[]),...(${allow(`${groups.manager}.getHostId()`, "null")}?[null,void 0]:[])].includes(${groups.account})`,
        },
        {
          // 26.10 起改成 if/return 语句体，账号结果先落到局部变量再比较。
          pattern: [
            `function ${id}\\((?<manager>${id}),\\{runtime:(?<runtime>${id}),storage:(?<storage>${id})\\},(?<before>${id}),${id}\\)\\{let (?<pending>${id})=null;`,
            `return\\{async readServiceTier\\((?<model>${id})\\)\\{for\\(;\\k<pending>!=null;\\)await \\k<pending>\\.catch\\(\\(\\)=>void 0\\);`,
            `return \\k<before>\\(\\),${id}\\(\\k<manager>\\.requestClient,\\k<manager>\\.logger,\\k<model>,async\\(\\)=>\\{`,
            `if\\(\\k<manager>\\.getHostId\\(\\)===${local}&&await \\k<storage>\\.readGlobalState\\(\`use-copilot-auth-if-available\`\\)===!0&&await \\k<runtime>\\.isCopilotApiAvailable\\(\\)\\)return!1;`,
            `let (?<result>${id})=await \\k<manager>\\.getAccount\\(\\{priority:${critical}\\}\\);`,
            `return (?<gate>\\k<result>\\.account\\?\\.type===${chatgpt}(?:\\|\\|\\k<result>\\.account\\?\\.type===${literal})*)\\}\\)\\},async writeServiceTier\\(`,
          ].join(""),
          distinct: ["manager", "result"],
          replacement: (groups) => `(${groups.gate}||${allow(`${groups.manager}.getHostId()`, `${groups.result}.account?.type`)})`,
        },
        {
          // 26.1021 将 Copilot 判定收进 runtime，不再向默认队列传入 storage。
          pattern: [
            `function ${id}\\((?<manager>${id}),\\{runtime:(?<runtime>${id})\\},(?<before>${id}),${id}\\)\\{let (?<pending>${id})=null;`,
            `return\\{async readServiceTier\\((?<model>${id})\\)\\{for\\(;\\k<pending>!=null;\\)await \\k<pending>\\.catch\\(\\(\\)=>void 0\\);`,
            `return \\k<before>\\(\\),${id}\\(\\k<manager>\\.requestClient,\\k<manager>\\.logger,\\k<model>,async\\(\\)=>\\{`,
            `if\\(\\k<manager>\\.getHostId\\(\\)===${local}&&await \\k<runtime>\\.isUsingCopilotInference\\(\\)\\)return!1;`,
            `let (?<result>${id})=await \\k<manager>\\.getAccount\\(\\{priority:${critical}\\}\\);`,
            `return (?<gate>\\k<result>\\.account\\?\\.type===${chatgpt}(?:\\|\\|\\k<result>\\.account\\?\\.type===${literal})*)\\}\\)\\},async writeServiceTier\\(`,
          ].join(""),
          distinct: ["manager", "result"],
          replacement: (groups) => `(${groups.gate}||${allow(`${groups.manager}.getHostId()`, `${groups.result}.account?.type`)})`,
        },
      ],
    },
    {
      kind: "queue-explicit-access",
      pattern: [
        `function ${id}\\((?<manager>${id}),\\{settings:(?<settings>${id}),storage:(?<storage>${id}),filesystem:${id},runtime:(?<runtime>${id})\\},(?<readDefault>${id}),${id},${id}\\)\\{`,
        `return async\\(${id},(?<message>${id})\\)=>\\{let (?<options>${id})=\\k<message>\\.submissionOptions,${id}=\\k<message>\\.context\\.workPermissions,${id}=${id}!=null,`,
        `(?<host>${id})=\\k<options>\\?\\.executionHostId\\?\\?\\k<manager>\\.getHostId\\(\\);`,
        `[\\s\\S]{0,1800}?let\\[${id},${id}\\]=await Promise\\.all\\(\\[\\(async\\(\\)=>\\{if\\(\\k<options>==null\\)return \\k<readDefault>\\(${id}\\?\\.latestModel\\|\\|null\\);`,
        `if\\(\\k<options>\\.serviceTier==null\\)return null;let\\{account:(?<account>${id})\\}=await \\k<manager>\\.getAccount\\(\\{priority:${critical}\\}\\);`,
        `return (?<gate>\\k<account>\\?\\.type===${chatgpt}(?:\\|\\|\\k<account>\\?\\.type===${literal})*)\\?\\k<options>\\.serviceTier:null\\}\\)\\(\\),\\k<options>\\?\\.usePermissionSelection\\?\\?\\k<settings>\\.readPermissionRefreshEnabled\\(\\)\\]\\)`,
      ].join(""),
      // 补丁文本在内层箭头函数里引用这些绑定；账号解构名与它们重名会被遮蔽，必须拒绝。
      distinct: ["manager", "storage", "runtime", "host", "account"],
      // manager 和实际执行端都必须是本机；所选 Copilot 不能借底层 API 身份放行。
      replacement: (groups) => `(${groups.gate}||${groups.host}===${local}&&${allow(`${groups.manager}.getHostId()`, `${groups.account}?.type`)}&&!(await ${groups.storage}.readGlobalState(\`use-copilot-auth-if-available\`)===!0&&await ${groups.runtime}.isCopilotApiAvailable()))`,
    },
  ];

  function plan(source, url) {
    const diagnostics = { url, reason: "ready", matches: {} };
    const failed = (reason) => ({ ok: false, replacements: [], diagnostics: { ...diagnostics, reason } });
    if (typeof source !== "string" || !source || typeof url !== "string" || !url.trim()) {
      return failed("invalid-input");
    }
    const replacements = [];
    for (const definition of gates) {
      // 同一门控的多个历史结构合并计数：任何版本只允许恰好命中一处。
      const matches = (definition.variants ?? [definition]).flatMap((variant) =>
        [...source.matchAll(new RegExp(variant.pattern, "g"))].map((match) => ({ match, variant })));
      diagnostics.matches[definition.kind] = matches.length;
      if (matches.length !== 1) continue;
      const { match, variant } = matches[0];
      const names = (variant.distinct ?? []).map((name) => match.groups[name]);
      if (new Set(names).size !== names.length) return failed("shadowed-bindings");
      const gate = match.groups.gate;
      const offset = match[0].indexOf(gate);
      if (offset < 0 || offset !== match[0].lastIndexOf(gate) || /[\r\n]/.test(gate)) {
        return failed("unsupported-shape");
      }
      const start = match.index + offset;
      const text = variant.replacement(match.groups);
      if (/[\r\n]/.test(text)) return failed("unsupported-shape");
      replacements.push({ kind: definition.kind, start, end: start + gate.length, text });
    }
    if (Object.values(diagnostics.matches).some((count) => count > 1)) return failed("ambiguous-gates");
    if (replacements.length !== gates.length) return failed("incomplete-gates");
    replacements.sort((left, right) => left.start - right.start);
    if (replacements.some((replacement, index) => index > 0 && replacement.start < replacements[index - 1].end)) {
      return failed("overlapping-gates");
    }
    return { ok: true, replacements, diagnostics };
  }

  function transform(source, url) {
    const result = plan(source, url);
    let patchedSource = source;
    if (result.ok) {
      for (const replacement of [...result.replacements].reverse()) {
        patchedSource = patchedSource.slice(0, replacement.start) + replacement.text + patchedSource.slice(replacement.end);
      }
    }
    return { ...result, patchedSource };
  }

  function moduleUrl(value) {
    if (typeof value !== "string" || /[\r\n]/.test(value)) return null;
    try {
      const parsed = new URL(value);
      return (parsed.protocol === "app:" && parsed.hostname === "-"
        || parsed.protocol === "file:" && !parsed.hostname) ? parsed : null;
    } catch { return null; }
  }

  // 只分词，不执行源码。模板的原始文本、注释和正则都不参与模块路径识别。
  // 正则字面量边界采用 js-tokens 4 的词法规则，并补充现代 d/v 标志。
  /*
   * Copyright (c) 2014, 2015, 2016, 2017, 2018 Simon Lydell
   * Permission is hereby granted, free of charge, to any person obtaining a copy
   * of this software and associated documentation files (the "Software"), to deal
   * in the Software without restriction, including without limitation the rights
   * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
   * copies of the Software, and to permit persons to whom the Software is
   * furnished to do so, subject to the following conditions:
   * The above copyright notice and this permission notice shall be included in
   * all copies or substantial portions of the Software.
   * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
   * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
   * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
   * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
   * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
   * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
   * SOFTWARE.
   */
  function moduleTokens(source) {
    const regex = /\/(?:\[(?:[^\]\\\r\n]|\\[^\r\n])*\]|[^\/\\\r\n[]|\\[^\r\n])+\/[dgimsuvy]*/y;
    const identifier = /[$A-Z_a-z\u0080-\uFFFF][$\w\u0080-\uFFFF]*/y;
    const number = /(?:0[xX][\da-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|(?:\d[\d_]*(?:\.[\d_]*)?|\.\d[\d_]*)(?:[eE][+-]?[\d_]+)?)n?/y;
    const operator = /(?:\?\.|\+\+|--|=>|\*\*|&&|\|\||\?\?|===|!==|==|!=|<=|>=|<<|>>>|>>|\.{3}|[+\-*%&|^!?<>=]=?)/y;
    let position = 0;
    let expressionAllowed = true;
    let last = null;
    const parentheses = [];
    const blocks = [];
    const emit = (token) => {
      const value = token.value;
      if (token.type === "name") expressionAllowed = /^(?:return|throw|case|delete|void|typeof|new|in|instanceof|yield|await|else|do)$/.test(value);
      else if (["string", "regex", "number", "template-end"].includes(token.type)) expressionAllowed = false;
      else if (value === "(") { parentheses.push(/^(?:if|while|for|with|switch|catch)$/.test(last?.value)); expressionAllowed = true; }
      else if (value === ")") expressionAllowed = parentheses.pop() === true;
      else if (value === "{") {
        blocks.push(!["=", "(", "[", ",", ":", "?", "return", "yield"].includes(last?.value));
        expressionAllowed = true;
      } else if (value === "}") expressionAllowed = blocks.pop() !== false;
      else expressionAllowed = ![".", "?.", "]", "++", "--"].includes(value);
      last = token;
      return token;
    };
    function* code(templateExpression = false) {
      let braces = 0;
      while (position < source.length) {
        const start = position;
        const char = source[position];
        if (/\s/.test(char)) { position++; continue; }
        if (source.startsWith("//", position)) {
          const end = source.indexOf("\n", position + 2);
          position = end < 0 ? source.length : end + 1;
          continue;
        }
        if (source.startsWith("/*", position)) {
          const end = source.indexOf("*/", position + 2);
          if (end < 0) throw new Error("unterminated-comment");
          position = end + 2;
          continue;
        }
        if (char === "'" || char === '"') {
          position++;
          for (; position < source.length; position++) {
            if (source[position] === "\\") { position++; continue; }
            if (source[position] === char) break;
            if (/[\r\n]/.test(source[position])) throw new Error("unterminated-string");
          }
          if (position >= source.length) throw new Error("unterminated-string");
          yield emit({ type: "string", start, end: ++position, value: source.slice(start, position) });
          continue;
        }
        if (char === "`") {
          position++;
          let interpolated = false;
          for (;;) {
            if (position >= source.length) throw new Error("unterminated-template");
            if (source[position] === "\\") { position += 2; continue; }
            if (source[position] === "`") {
              position++;
              yield emit({ type: interpolated ? "template-end" : "string", start, end: position, value: source.slice(start, position) });
              break;
            }
            if (source.startsWith("${", position)) {
              if (!interpolated) yield emit({ type: "template-start", start, end: position, value: "`" });
              interpolated = true;
              position += 2;
              expressionAllowed = true;
              yield* code(true);
              continue;
            }
            position++;
          }
          continue;
        }
        if (char === "/" && expressionAllowed) {
          regex.lastIndex = position;
          const match = regex.exec(source);
          if (match) {
            position = regex.lastIndex;
            yield emit({ type: "regex", start, end: position, value: match[0] });
            continue;
          }
        }
        identifier.lastIndex = position;
        const name = identifier.exec(source);
        if (name) {
          position = identifier.lastIndex;
          yield emit({ type: "name", start, end: position, value: name[0] });
          continue;
        }
        number.lastIndex = position;
        const numeric = number.exec(source);
        if (numeric) {
          position = number.lastIndex;
          yield emit({ type: "number", start, end: position, value: numeric[0] });
          continue;
        }
        operator.lastIndex = position;
        const operation = operator.exec(source);
        if (operation) {
          position = operator.lastIndex;
          yield emit({ type: "punctuator", start, end: position, value: operation[0] });
          continue;
        }
        position++;
        if (char === "{") braces++;
        if (char === "}") {
          if (templateExpression && braces === 0) return;
          braces--;
        }
        yield emit({ type: "punctuator", start, end: position, value: char });
      }
      if (templateExpression) throw new Error("unterminated-template-expression");
    }
    return code();
  }

  function stringValue(token) {
    // 模块 specifier 一律静态解析，禁止 eval / Function；不认识的转义直接拒绝。
    const text = token.value.slice(1, -1);
    let value = "";
    for (let index = 0; index < text.length; index++) {
      const char = text[index];
      if (char !== "\\") { value += char; continue; }
      const escaped = text[++index];
      if (escaped == null) throw new Error("invalid-module-string");
      if (escaped === "\r" || escaped === "\n") {
        if (escaped === "\r" && text[index + 1] === "\n") index++;
        continue;
      }
      const simple = { n: "\n", r: "\r", t: "\t", b: "\b", f: "\f", v: "\v", "0": "\0" };
      if (Object.hasOwn(simple, escaped)) { value += simple[escaped]; continue; }
      if (escaped === "x" || escaped === "u") {
        const rest = text.slice(index + 1);
        const match = escaped === "x" ? /^[\da-fA-F]{2}/.exec(rest)
          : /^(?:[\da-fA-F]{4}|\{[\da-fA-F]+\})/.exec(rest);
        if (!match) throw new Error("invalid-module-string");
        const hex = match[0].replace(/[{}]/g, "");
        value += String.fromCodePoint(parseInt(hex, 16));
        index += match[0].length;
      } else if (/[1-9]/.test(escaped)) throw new Error("invalid-module-string");
      else value += escaped;
    }
    return value;
  }

  function rewriteModuleBase(source, url) {
    const iterator = moduleTokens(source);
    const buffered = [];
    const edits = [];
    const counts = { staticImports: 0, dynamicImports: 0, importMetaUrls: 0 };
    let previous = null;
    const peek = (offset = 0) => {
      while (buffered.length <= offset) {
        const next = iterator.next();
        buffered.push(next.done ? null : next.value);
      }
      return buffered[offset];
    };
    const take = () => { const token = peek(); buffered.shift(); if (token) previous = token; return token; };
    const replaceString = (token, kind) => {
      const specifier = stringValue(token);
      counts[kind]++;
      if (specifier.startsWith(".") || specifier.startsWith("/")) {
        edits.push({ start: token.start, end: token.end, text: JSON.stringify(new URL(specifier, url).href) });
      }
    };
    for (;;) {
      const before = previous;
      const token = take();
      if (!token) break;
      if (token.type !== "name" || [".", "?."].includes(before?.value)) continue;
      if (token.value === "import") {
        if ([":", ",", "}"].includes(peek()?.value)) continue;
        if (peek()?.value === ".") {
          if (peek(1)?.value !== "meta" || peek(2)?.value !== "." || peek(3)?.value !== "url") {
            throw new Error("unsupported-import-meta");
          }
          take(); take(); take();
          const end = take().end;
          edits.push({ start: token.start, end, text: JSON.stringify(url) });
          counts.importMetaUrls++;
        } else if (peek()?.value === "(") {
          // 对象和类的 import(...) 方法名不是 ImportExpression。
          if (peek(1)?.type !== "string") {
            let depth = 0;
            let offset = 0;
            do {
              const part = peek(offset++);
              if (!part) break;
              if (part.value === "(") depth++;
              if (part.value === ")") depth--;
            } while (depth > 0);
            if (depth === 0 && peek(offset)?.value === "{") continue;
          }
          take();
          const argument = take();
          if (argument?.type !== "string" || ![",", ")"].includes(peek()?.value)) {
            throw new Error("nonliteral-dynamic-import");
          }
          replaceString(argument, "dynamicImports");
        } else {
          let specifier = take();
          if (specifier?.type !== "string") {
            while (specifier && specifier.value !== "from" && specifier.value !== ";") specifier = take();
            specifier = specifier?.value === "from" ? take() : null;
          }
          if (specifier?.type !== "string") throw new Error("unsupported-static-import");
          replaceString(specifier, "staticImports");
        }
      } else if (token.value === "export" && ["*", "{"].includes(peek()?.value)) {
        if (peek().value === "{") {
          while (peek() && take().value !== "}") { /* 只消费 export specifier 列表。 */ }
          if (peek()?.value !== "from") continue;
        } else {
          while (peek() && peek().value !== "from" && peek().value !== ";") take();
        }
        if (take()?.value !== "from") throw new Error("unsupported-static-export");
        const specifier = take();
        if (specifier?.type !== "string") throw new Error("unsupported-static-export");
        replaceString(specifier, "staticImports");
      }
    }
    const chunks = [];
    let offset = 0;
    for (const edit of edits) {
      if (edit.start < offset) throw new Error("overlapping-module-edits");
      chunks.push(source.slice(offset, edit.start), edit.text);
      offset = edit.end;
    }
    chunks.push(source.slice(offset));
    return { source: chunks.join(""), counts };
  }

  // 入口模块（app-initial）追加执行完成标记；启动守护和重载探针都只看这一个标记。
  function entryModuleSource(source, url) {
    return `${source}\n;window.__codexPlusNativeFastCompiled="v1";window.__codexPlusNativeFastPatchState="patched";window.__codexPlusNativeFastModuleComplete?.();\n//# sourceURL=${url}\n`;
  }

  function prepareModule(source, url) {
    const failed = (reason, diagnostics = {}) => ({ ok: false, patchedSource: source, diagnostics: { url, ...diagnostics, reason } });
    if (!runtime.status().enabled) return failed("disabled");
    if (!moduleUrl(url)) return failed("unsupported-module-url");
    const transformed = transform(source, url);
    if (!transformed.ok) return transformed;
    try {
      const rewritten = rewriteModuleBase(transformed.patchedSource, url);
      return {
        ...transformed,
        patchedSource: entryModuleSource(rewritten.source, url),
        diagnostics: { ...transformed.diagnostics, ...rewritten.counts, reason: "module-prepared" },
      };
    } catch (error) {
      return failed(error instanceof Error ? error.message : "unsupported-module-syntax", transformed.diagnostics);
    }
  }

  // ---------- 多改写器模块补丁通道 ----------
  // 每个改写器声明自己要改的目标块（按文件名前缀识别）与各自的开关；同一块可被多个改写器修改。
  // 每个改写器对自己负责的块全有或全无：任何一个目标块结构不符，就整体放弃该改写器，
  // 不影响其它改写器，也不会留下半套补丁。
  const entryPrefix = "app-initial-";
  const moduleSourceLimit = 24 * 1024 * 1024;
  const rewriters = root.__codexPlusNativeModuleRewriters instanceof Map ? root.__codexPlusNativeModuleRewriters : new Map();
  root.__codexPlusNativeModuleRewriters = rewriters;

  function registerRewriter(rewriter) {
    if (!rewriter || typeof rewriter.id !== "string" || !Array.isArray(rewriter.targets)
      || typeof rewriter.plan !== "function" || typeof rewriter.enabled !== "function") return false;
    rewriters.set(rewriter.id, rewriter);
    return true;
  }

  registerRewriter({
    id: "fast",
    targets: [{ name: "app-initial", prefix: entryPrefix, unique: true }],
    enabled: () => runtime.status().enabled,
    plan: (source, url) => plan(source, url),
  });

  function moduleFileName(url) {
    try {
      return new URL(url).pathname.split("/").pop() || "";
    } catch {
      return "";
    }
  }

  function matchesTarget(target, url) {
    const name = moduleFileName(url);
    return name.startsWith(target.prefix) && /^[\w-]+\.js$/.test(name.slice(target.prefix.length));
  }

  function activeRewriters() {
    return [...rewriters.values()].filter((rewriter) => {
      try { return rewriter.enabled() === true; } catch { return false; }
    });
  }

  function planRewriter(rewriter, source, url, target) {
    try {
      const result = rewriter.plan(source, url, target.name);
      return result && typeof result === "object" ? result : { ok: false, diagnostics: { reason: "invalid-plan" } };
    } catch {
      return { ok: false, diagnostics: { reason: "plan-failed" } };
    }
  }

  function overlapping(replacements) {
    const sorted = [...replacements].sort((left, right) => left.start - right.start || left.end - right.end);
    return sorted.some((item, index) => index > 0 && item.start < sorted[index - 1].end);
  }

  function applyReplacements(source, replacements) {
    let patched = source;
    for (const replacement of [...replacements].sort((left, right) => right.start - left.start || right.end - left.end)) {
      patched = patched.slice(0, replacement.start) + replacement.text + patched.slice(replacement.end);
    }
    return patched;
  }

  // Fetch 拦截模式逐个响应改写：只应用开启且结构完整的改写器。
  function transformModule(source, url) {
    const diagnostics = { url, reason: "ready", rewriters: {} };
    const unchanged = (reason) => ({ ok: false, patchedSource: source, applied: [], diagnostics: { ...diagnostics, reason } });
    if (typeof source !== "string" || !source || typeof url !== "string" || !url.trim()) return unchanged("invalid-input");
    let replacements = [];
    const applied = [];
    for (const rewriter of activeRewriters()) {
      const target = rewriter.targets.find((item) => matchesTarget(item, url));
      if (!target) continue;
      const result = planRewriter(rewriter, source, url, target);
      diagnostics.rewriters[rewriter.id] = result.ok === true ? "patched" : result.diagnostics?.reason ?? "unsupported";
      if (result.ok !== true || !Array.isArray(result.replacements)) continue;
      const next = [...replacements, ...result.replacements];
      if (overlapping(next)) {
        diagnostics.rewriters[rewriter.id] = "overlapping-rewriters";
        continue;
      }
      replacements = next;
      applied.push(rewriter.id);
    }
    if (!applied.length) return unchanged("no-applicable-rewriter");
    return { ok: true, patchedSource: applyReplacements(source, replacements), applied, diagnostics };
  }

  // app:// 模式在文档开始前一次性准备全部模块，因此可以跨块校验每个改写器的完整性。
  function prepareModules(modules) {
    const diagnostics = { reason: "modules-prepared", rewriters: {}, modules: 0 };
    const failed = (reason) => ({ ok: false, modules: [], diagnostics: { ...diagnostics, reason } });
    if (!Array.isArray(modules)) return failed("invalid-input");
    const sources = modules.filter((item) => item && typeof item.url === "string" && typeof item.source === "string"
      && item.source.length > 0 && item.source.length <= moduleSourceLimit && moduleUrl(item.url));
    const active = activeRewriters();
    if (!active.length) return failed("disabled");
    const accepted = new Map();
    for (const rewriter of active) {
      const perModule = new Map();
      let reason = "patched";
      for (const target of rewriter.targets) {
        const candidates = sources.filter((item) => matchesTarget(target, item.url));
        if (!candidates.length) { reason = `missing-${target.name}`; break; }
        if (target.unique && candidates.length !== 1) { reason = `ambiguous-${target.name}`; break; }
        const results = candidates.map((item) => ({ item, result: planRewriter(rewriter, item.source, item.url, target) }));
        const matched = results.filter(({ result }) => result.ok === true);
        // 同名前缀的其它块必须完全不含目标结构；部分命中说明结构已变化，不能猜。
        const partial = results.some(({ result }) => result.ok !== true && Number(result.diagnostics?.matched) > 0);
        if (matched.length !== 1 || partial) {
          reason = matched.length > 1 || partial
            ? `ambiguous-${target.name}`
            : `${target.name}:${results[0]?.result?.diagnostics?.reason ?? "unsupported"}`;
          break;
        }
        perModule.set(matched[0].item.url, matched[0].result.replacements);
      }
      if (reason === "patched") {
        for (const [url, replacements] of perModule) {
          const existing = [...accepted.values()].flatMap((other) => other.get(url) ?? []);
          if (overlapping([...existing, ...replacements])) { reason = "overlapping-rewriters"; break; }
        }
      }
      diagnostics.rewriters[rewriter.id] = reason;
      if (reason === "patched") accepted.set(rewriter.id, perModule);
    }
    if (!accepted.size) return failed("unsupported");
    const urls = new Set([...accepted.values()].flatMap((perModule) => [...perModule.keys()]));
    const entry = [...urls].find((url) => moduleFileName(url).startsWith(entryPrefix));
    if (!entry) return failed("entry-not-patched");
    const prepared = [];
    for (const url of urls) {
      const source = sources.find((item) => item.url === url).source;
      const replacements = [...accepted.values()].flatMap((perModule) => perModule.get(url) ?? []);
      try {
        const rewritten = rewriteModuleBase(applyReplacements(source, replacements), url);
        prepared.push({
          url,
          patchedSource: url === entry ? entryModuleSource(rewritten.source, url) : `${rewritten.source}\n//# sourceURL=${url}\n`,
        });
      } catch (error) {
        return failed(error instanceof Error ? error.message : "unsupported-module-syntax");
      }
    }
    // 入口放在第一位，便于新文档按顺序登记 import map 与预加载。
    prepared.sort((left, right) => (left.url === entry ? -1 : right.url === entry ? 1 : 0));
    diagnostics.modules = prepared.length;
    return { ok: true, modules: prepared, diagnostics };
  }

  async function defaultFetchText(url) {
    const response = await root.fetch(url);
    if (!response.ok) return null;
    const text = await response.text();
    return text.length > moduleSourceLimit ? null : text;
  }

  // 从入口脚本沿 Vite 依赖表找目标块；只顺着改写器声明的前缀走，不遍历整个资源目录。
  async function discoverModules(entries, active, fetchText = defaultFetchText) {
    const follow = new Set(active.flatMap((rewriter) => rewriter.targets.flatMap((target) => [target.prefix, ...(target.via ?? [])])));
    const queue = entries.map((url) => ({ url, depth: 0 }));
    const seen = new Set();
    const found = [];
    while (queue.length && seen.size < 24) {
      const { url, depth } = queue.shift();
      if (seen.has(url)) continue;
      seen.add(url);
      let source = null;
      try { source = await fetchText(url); } catch { source = null; }
      if (typeof source !== "string") continue;
      found.push({ url, source });
      if (depth >= 4) continue;
      for (const match of source.matchAll(/["'`]\.\/([\w-]+\.js)["'`]/g)) {
        const name = match[1];
        if (![...follow].some((prefix) => name.startsWith(prefix))) continue;
        try { queue.push({ url: new URL(`./${name}`, url).href, depth: depth + 1 }); } catch { /* 忽略非法地址。 */ }
      }
    }
    return found;
  }

  async function prepareDocumentModules(options = {}) {
    const location = options.location ?? root.location;
    const document = options.document ?? root.document;
    if (location?.protocol !== "app:" || location?.host !== "-") {
      return { ok: false, modules: [], diagnostics: { reason: "unsupported-document-url", rewriters: {} } };
    }
    const active = activeRewriters();
    if (!active.length) return { ok: false, modules: [], diagnostics: { reason: "disabled", rewriters: {} } };
    const entries = [...(document?.scripts ?? [])].map((script) => script.src).filter((src) => {
      try {
        const parsed = new URL(src);
        // app:// 是 Electron 注册的标准协议；按协议 + 主机比较，不依赖非特殊协议的 origin 序列化。
        return parsed.protocol === location.protocol && parsed.host === location.host
          && /^\/assets\/index-[\w-]+\.js$/.test(parsed.pathname);
      } catch {
        return false;
      }
    }).slice(0, 4);
    const discovered = await discoverModules(entries, active, options.fetchText);
    return prepareModules(discovered);
  }

  function installModule(patchedSource, url) {
    return installModules([{ url, patchedSource }]);
  }

  function installModules(modules, details = {}) {
    const entryModule = Array.isArray(modules) ? modules.find((item) => moduleFileName(item?.url).startsWith(entryPrefix)) ?? modules[0] : null;
    const url = entryModule?.url;
    const patchedSource = entryModule?.patchedSource;
    const diagnostics = { url, reason: "module-preloaded", modules: Array.isArray(modules) ? modules.length : 0 };
    if (details && typeof details === "object" && details.rewriters && typeof details.rewriters === "object") {
      diagnostics.rewriters = { ...details.rewriters };
    }
    const failed = (reason) => {
      diagnostics.reason = reason;
      root.__codexPlusNativeFastPatchState = "failed";
      root.__codexPlusNativeFastModuleDiagnostics = diagnostics;
      return { ok: false, diagnostics };
    };
    if (!activeRewriters().length) return { ok: false, diagnostics: { url, reason: "disabled" } };
    const document = root.document;
    if (!entryModule || !moduleUrl(url) || !moduleUrl(root.location?.href)
      || !modules.every((item) => moduleUrl(item?.url))) return { ok: false, diagnostics: { url, reason: "unsupported-document-url" } };
    if (!document || document.documentElement) return failed("document-already-started");
    if (typeof patchedSource !== "string" || !patchedSource.includes('window.__codexPlusNativeFastCompiled="v1";')
      || !modules.every((item) => typeof item.patchedSource === "string" && item.patchedSource)) return failed("unprepared-module");
    let temporaryRoot = null;
    try {
      // import map 不能撤销。预加载失败时仅重载一次，下一文档跳过映射并正常加载原模块。
      const fallbackKey = "codex-plus-native-fast-fallback-v1";
      const storage = root.sessionStorage;
      if (!storage) return failed("fallback-storage-unavailable");
      if (storage.getItem(fallbackKey) === url) {
        storage.removeItem(fallbackKey);
        return failed("module-preload-failed-open");
      }
      storage.setItem(fallbackKey, "checking");
      storage.removeItem(fallbackKey);
      if (root.HTMLScriptElement?.supports?.("importmap") !== true) return failed("importmap-unavailable");
      const map = document.createElement("script");
      // 入口在前：所有补丁块都必须在 meta CSP 解析前登记映射并开始预加载。
      const ordered = [entryModule, ...modules.filter((item) => item !== entryModule)];
      const preloads = ordered.map(() => document.createElement("link"));
      if (preloads.some((preload) => preload.relList?.supports?.("modulepreload") !== true)) return failed("modulepreload-unavailable");
      const imports = {};
      ordered.forEach((item, index) => {
        const blob = root.URL.createObjectURL(new root.Blob([item.patchedSource], { type: "text/javascript" }));
        imports[item.url] = blob;
        preloads[index].rel = "modulepreload";
        preloads[index].href = blob;
      });
      map.type = "importmap";
      map.textContent = JSON.stringify({ imports });
      let watchdog = null;
      let recovering = false;
      const cleanup = () => {
        if (watchdog != null) { root.clearTimeout(watchdog); watchdog = null; }
        document.removeEventListener("DOMContentLoaded", armWatchdog);
        if (root.__codexPlusNativeFastModuleComplete === complete) delete root.__codexPlusNativeFastModuleComplete;
      };
      // 懒加载块的预加载失败会让之后的 import 直接报错，即使入口已执行也要回退一次。
      const recover = (reason, lazyModule = false) => {
        if (recovering || !lazyModule && root.__codexPlusNativeFastCompiled === "v1") return;
        recovering = true;
        cleanup();
        failed(reason);
        try {
          storage.setItem(fallbackKey, url);
          root.location.reload();
        } catch { /* 存储失败时禁止重载，避免重载循环。 */ }
      };
      const complete = () => {
        if (root.__codexPlusNativeFastCompiled === "v1") cleanup();
      };
      const armWatchdog = () => {
        if (root.__codexPlusNativeFastCompiled === "v1") { cleanup(); return; }
        if (watchdog != null) return;
        // 不监听普通 error/unhandledrejection。只检查目标模块自己的执行完成标记。
        watchdog = root.setTimeout(() => {
          watchdog = null;
          recover("module-execution-timeout");
        }, 30000);
      };
      preloads.forEach((preload, index) => {
        preload.onerror = () => recover("module-preload-failed", index > 0);
      });
      temporaryRoot = document.createElement("html");
      document.appendChild(temporaryRoot);
      temporaryRoot.appendChild(map);
      preloads.forEach((preload) => temporaryRoot.appendChild(preload));
      root.__codexPlusNativeFastModuleComplete = complete;
      document.addEventListener("DOMContentLoaded", armWatchdog, { once: true });
      if (document.readyState !== "loading") armWatchdog();
      root.__codexPlusNativeFastModuleDiagnostics = diagnostics;
      return { ok: true, diagnostics };
    } catch {
      return failed("module-install-failed");
    } finally {
      // 必须同步移除临时根；保留它会让原 HTML parser 的 head/body 脱离文档。
      if (temporaryRoot?.parentNode === document) document.removeChild(temporaryRoot);
    }
  }

  const api = {
    plan, transform, prepareModule, installModule, runtime,
    registerRewriter, transformModule, prepareModules, discoverModules, prepareDocumentModules, installModules,
  };
  if (typeof module === "object" && module.exports) module.exports = api;
  if (typeof window === "object") window.__codexPlusNativeFastUnlock = api;
})();
