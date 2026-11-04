(() => {
  const root = typeof window === "object" ? window : {};
  const id = "[A-Za-z_$][\\w$]*";
  const call = `\\(0,${id}\\.${id}\\)`;
  const literal = '"[A-Za-z]\\w*"';
  const allow = (host, auth) => `window.__codexPlusNativeFastUnlock?.runtime?.allow?.(${host},${auth})===true`;
  const gates = [
    {
      kind: "ui-access",
      pattern: [
        `(?<host>${id})=${id}\\?\\.hostId\\?\\?${id},(?<identity>${id})=${call}\\(\\k<host>\\),`,
        `(?<variable>${id})=(?<gate>\\k<identity>\\?\\.authMethod==="chatgpt"\\|\\|\\k<identity>\\?\\.authMethod==="personalAccessToken"(?:\\|\\|\\k<identity>\\?\\.authMethod===${literal})*),`,
        `(?<method>${id})=\\k<identity>\\?\\.authMethod\\?\\?null;[^{}]{0,300}\\{authMethod:\\k<method>,hostId:\\k<host>\\}`,
        `[^]{0,1500}?serviceTierAccess:`,
      ].join(""),
      distinct: ["host", "identity"],
      replacement: g => `(${g.gate}||${allow(g.host, `${g.identity}?.authMethod`)})`,
    },
    {
      kind: "request-access",
      pattern: [
        `async function ${id}\\((?<scope>${id}),(?<host>${id})\\)\\{let (?<auth>${id})=await ${call}\\(\\k<scope>,\\k<host>\\);`,
        `if\\((?<gate>"chatgpt"!==\\k<auth>&&"personalAccessToken"!==\\k<auth>(?:&&${literal}!==\\k<auth>)*)\\)return null;`,
        `let (?<requirements>${id})=await ${call}\\(\\k<scope>,\\k<host>,\\{priority:"critical"\\}\\);`,
        `return \\k<scope>\\.query\\.setData\\(${id}\\.${id},\\{authMethod:\\k<auth>,hostId:\\k<host>\\},\\k<requirements>\\),${call}\\(\\k<requirements>\\)\\}`,
      ].join(""),
      distinct: ["scope", "host", "auth"],
      replacement: g => `${g.gate}&&!(${allow(g.host, g.auth)})`,
    },
    {
      kind: "queue-default-access",
      pattern: [
        `function\\((?<manager>${id}),\\{runtime:(?<runtime>${id})\\},(?<before>${id}),${id}\\)\\{let (?<pending>${id})=null;`,
        `return\\{async readServiceTier\\((?<model>${id})\\)\\{for\\(;null!=\\k<pending>;\\)await \\k<pending>\\.catch\\(\\(\\)=>void 0\\);`,
        `return \\k<before>\\(\\),${id}\\(\\k<manager>\\.requestClient,\\k<manager>\\.logger,\\k<model>,async\\(\\)=>\\{`,
        `if\\("local"===\\k<manager>\\.getHostId\\(\\)&&await \\k<runtime>\\.isUsingCopilotInference\\(\\)\\)return!1;`,
        `let (?<result>${id})=await \\k<manager>\\.getAccount\\(\\{priority:"critical"\\}\\);`,
        `return (?<gate>\\k<result>\\.account\\?\\.type==="chatgpt"(?:\\|\\|\\k<result>\\.account\\?\\.type===${literal})*)\\}\\)\\},async writeServiceTier\\(`,
      ].join(""),
      distinct: ["manager", "result"],
      replacement: g => `(${g.gate}||${allow(`${g.manager}.getHostId()`, `${g.result}.account?.type`)})`,
    },
    {
      kind: "queue-explicit-access",
      pattern: [
        `function\\((?<manager>${id}),\\{settings:(?<settings>${id}),storage:${id},filesystem:${id},runtime:(?<runtime>${id})\\},(?<readDefault>${id}),${id},${id}\\)\\{`,
        `return async\\(${id},(?<message>${id})\\)=>\\{let (?<options>${id})=\\k<message>\\.submissionOptions,${id}=\\k<message>\\.context\\.workPermissions,${id}=null!=${id},`,
        `(?<host>${id})=\\k<options>\\?\\.executionHostId\\?\\?\\k<manager>\\.getHostId\\(\\);`,
        `[^]{0,2400}?let (?<readTier>${id})=async\\(\\)=>\\{if\\(null==\\k<options>\\)return \\k<readDefault>\\(${id}\\?\\.latestModel\\|\\|null\\);`,
        `if\\(null==\\k<options>\\.serviceTier\\)return null;let\\{account:(?<account>${id})\\}=await \\k<manager>\\.getAccount\\(\\{priority:"critical"\\}\\);`,
        `return (?<gate>\\k<account>\\?\\.type==="chatgpt"(?:\\|\\|\\k<account>\\?\\.type===${literal})*)\\?\\k<options>\\.serviceTier:null\\},\\[${id},${id}\\]=await Promise\\.all\\(\\[\\k<readTier>\\(\\),\\k<options>\\?\\.usePermissionSelection\\?\\?\\k<settings>\\.readPermissionRefreshEnabled\\(\\)\\]\\)`,
      ].join(""),
      distinct: ["manager", "host", "runtime", "account"],
      replacement: g => `(${g.gate}||${g.host}==="local"&&${allow(`${g.manager}.getHostId()`, `${g.account}?.type`)}&&typeof ${g.runtime}.isUsingCopilotInference==="function"&&!(await ${g.runtime}.isUsingCopilotInference()))`,
    },
  ];

  function isRuntime(source) {
    return typeof source === "string" && source.includes("as __webpack_require__")
      && source.includes(".__rspack_esm_ids") && source.includes(".__webpack_modules__");
  }

  function relevant(source) {
    return source.includes("readServiceTier") || source.includes("serviceTierAccess:") && source.includes("authMethod")
      || source.includes("query.setData") && source.includes("personalAccessToken");
  }

  async function readFactories(url, attempts = 50) {
    // 只读取当前文档已经执行的入口的模块表，不执行工厂，也不扫描所有懒加载分块。
    const namespace = await import(url);
    const modules = namespace.__webpack_require__?.m;
    if (!modules || typeof modules !== "object") throw new Error("rspack-modules-unavailable");
    for (let attempt = 0; ; attempt++) {
      const factories = Object.entries(modules).flatMap(([key, factory]) => {
        if (typeof factory !== "function") return [];
        const original = factory.__codexPlusNativeFastOriginal ?? factory;
        if (typeof original !== "function") return [];
        const source = Function.prototype.toString.call(original);
        return relevant(source) ? [{ key, source }] : [];
      });
      // 冷启动时入口已经执行，界面分块仍可能在加载；短暂等待，不执行工厂或触发额外请求。
      const planned = planFactories(factories);
      if (planned.ok || attempt + 1 >= attempts
        || !["incomplete-gates", "invalid-factories"].includes(planned.diagnostics.reason)) return factories;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }

  function planFactories(factories) {
    const diagnostics = { reason: "ready", matches: {} };
    const failed = reason => ({ ok: false, factories: [], diagnostics: { ...diagnostics, reason } });
    if (!Array.isArray(factories) || !factories.length || factories.length > 64) return failed("invalid-factories");
    const keys = new Set();
    let bytes = 0;
    for (const factory of factories) {
      const { key, source } = factory ?? {};
      if (typeof key !== "string" || !new RegExp(`^${id}$`).test(key) || keys.has(key)
        || typeof source !== "string" || !source.startsWith(`${key}(`)
        || !new RegExp(`^${id}\\(${id}(?:,${id}){1,2}\\)\\{`).test(source)) return failed("invalid-factories");
      keys.add(key);
      bytes += source.length;
      if (bytes > 24 * 1024 * 1024) return failed("factories-too-large");
    }
    const replacements = new Map();
    for (const gate of gates) {
      const matches = factories.flatMap(factory => [...factory.source.matchAll(new RegExp(gate.pattern, "g"))]
        .map(match => ({ factory, match })));
      diagnostics.matches[gate.kind] = matches.length;
      if (matches.length !== 1) continue;
      const { factory, match } = matches[0];
      const g = match.groups;
      const bindings = gate.distinct.map(name => g[name]);
      if (new Set(bindings).size !== bindings.length) return failed("shadowed-bindings");
      const offset = match[0].indexOf(g.gate);
      if (offset < 0 || offset !== match[0].lastIndexOf(g.gate)) return failed("unsupported-shape");
      const edits = replacements.get(factory.key) ?? [];
      edits.push({ start: match.index + offset, end: match.index + offset + g.gate.length, text: gate.replacement(g) });
      replacements.set(factory.key, edits);
    }
    if (Object.values(diagnostics.matches).some(n => n > 1)) return failed("ambiguous-gates");
    if (Object.values(diagnostics.matches).some(n => n !== 1)) return failed("incomplete-gates");
    const patched = [];
    for (const factory of factories) {
      const edits = replacements.get(factory.key);
      if (!edits) continue;
      if (/\bimport\s*[.(]/.test(factory.source)) return failed("unsupported-factory-import");
      edits.sort((a, b) => b.start - a.start);
      let source = factory.source;
      for (let i = 0; i < edits.length; i++) {
        const edit = edits[i];
        if (i && edit.end > edits[i - 1].start) return failed("overlapping-gates");
        source = source.slice(0, edit.start) + edit.text + source.slice(edit.end);
      }
      patched.push({ key: factory.key, source });
    }
    return { ok: true, factories: patched, diagnostics };
  }

  function registrationChunk(source, url) {
    if (!isRuntime(source)) return null;
    const register = new RegExp(`(?<chunk>${id})\\.__rspack_esm_ids,(?<modules>${id})=\\k<chunk>\\.__webpack_modules__,${id}=\\k<chunk>\\.__rspack_esm_runtime,${id}=0;for\\((?<key>${id}) in \\k<modules>\\)(?<require>${id})\\.o\\(\\k<modules>,\\k<key>\\)&&\\((?<assignment>\\k<require>\\.m\\[\\k<key>\\]=\\k<modules>\\[\\k<key>\\])\\)`, "g");
    const registrations = [...source.matchAll(register)];
    if (registrations.length !== 1) return null;
    const { groups: g } = registrations[0];
    const requireName = g.require.replace(/\$/g, "\\$");
    const dependency = new RegExp(`import\\*as (?<binding>${id}) from"(?<path>\\./\\d+\\.[a-f\\d]+\\.js)";${requireName}\\.C\\(\\k<binding>\\);`).exec(source);
    return dependency ? new URL(dependency.groups.path, url).href : null;
  }

  function prepare(source, url, factories, rewriteModuleBase, entryModuleSource) {
    const failed = reason => ({ ok: false, modules: [], diagnostics: { reason, rewriters: { fast: reason } } });
    if (typeof source !== "string" || source.length > 24 * 1024 * 1024
      || !/export const __rspack_esm_id=\d+;export const __rspack_esm_ids=\[\d+\];export const __webpack_modules__=\{/.test(source)
      || source.includes("__rspack_esm_runtime") || source.includes("__codexPlusFastFactories")) return failed("unsupported-rspack-chunk");
    const planned = planFactories(factories);
    if (!planned.ok) return { ...failed(planned.diagnostics.reason), diagnostics: { ...planned.diagnostics, rewriters: { fast: planned.diagnostics.reason } } };
    // script[src] 不走 import map；入口的静态依赖会走。依赖的原生 runtime 回调在入口启动前
    // 安装模块表代理，所有后续分块（包括重复工厂）仍由原生注册器处理，不改加载器或模块缓存。
    const hook = `\nexport const __rspack_esm_runtime = (__codexPlusRequire) => {
      const __codexPlusFastFactories={${planned.factories.map(f => f.source).join(",")}};
      __codexPlusRequire.m=new Proxy(__codexPlusRequire.m,{
        set(target,key,value,receiver){
          const replacement=Object.prototype.hasOwnProperty.call(__codexPlusFastFactories,key)?__codexPlusFastFactories[key]:null;
          if(replacement){replacement.__codexPlusNativeFastOriginal=value;value=replacement;}
          return Reflect.set(target,key,value,receiver);
        }
      });
      for(const key of Object.keys(__codexPlusFastFactories)){
        if(Object.prototype.hasOwnProperty.call(__codexPlusRequire.m,key)) __codexPlusRequire.m[key]=__codexPlusRequire.m[key];
      }
      ${entryModuleSource("", url)}
    };\n`;
    try {
      const rewritten = rewriteModuleBase(source, url);
      return {
        ok: true,
        modules: [{ url, patchedSource: rewritten.source + hook }],
        diagnostics: { reason: "modules-prepared", rewriters: { fast: "patched" }, matches: planned.diagnostics.matches, factories: planned.factories.map(f => f.key) },
      };
    } catch (error) {
      return failed(error instanceof Error ? error.message : "unsupported-rspack-syntax");
    }
  }

  const api = { isRuntime, readFactories, planFactories, registrationChunk, prepare };
  if (typeof module === "object" && module.exports) module.exports = api;
  if (typeof window === "object") window.__codexPlusNativeFastRspack = api;
})();
