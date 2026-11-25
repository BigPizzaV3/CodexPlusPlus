  async function loadBackendSettings() {
    const loaded = await loadBackendSettingsState();
    if (loaded && codexRemoteSessionProviderOverrideEnabled()) {
      void loadCodexModelCatalog();
    }
    refreshCodexPlusBackendToggles();
    if (loaded) syncOfficialUsagePolicy();
    // 后端可用后预取设置项清单，首次打开设置页时不必再等一次请求。
    if (loaded && typeof loadCodexPlusSettingsCatalog === "function") void loadCodexPlusSettingsCatalog();
    // 跟着设置走的功能（打字特效、桌宠、容量重试、梦皮 #2680、微信同步……）由各自的内置拓展订阅。
    if (loaded) emitCodexPlusSettingsLoaded("startup");
    return loaded;
  }

  /**
   * 读到后端设置（启动或心跳）后发公开事件 `settings/loaded`，参数 `{ reason: "startup" | "heartbeat", changedKeys? }`。
   * 内置拓展与第三方拓展都经 ctx.on 订阅，拓展卸下时订阅跟着撤掉。
   * typeof 守卫：本分片会被切片到不带拓展内核的沙箱里执行。
   */
  function emitCodexPlusSettingsLoaded(reason, changedKeys) {
    if (typeof codexPlusEmitExtensionEvent !== "function") return;
    // 心跳带上这次变了的字段名（管理器里改的设置靠它识别）；启动时第一次读到设置不带。
    const detail = Array.isArray(changedKeys) ? { reason, changedKeys } : { reason };
    codexPlusEmitExtensionEvent("settings/loaded", [detail]);
  }

  /** 两份设置之间值变了的顶层字段名。只给名字不给值，免得经事件把设置内容交给拓展。 */
  function codexPlusChangedBackendSettingKeys(previous, next) {
    const keys = new Set([...Object.keys(previous || {}), ...Object.keys(next || {})]);
    const changed = [];
    for (const key of keys) {
      let before;
      let after;
      try {
        before = JSON.stringify(previous?.[key]);
        after = JSON.stringify(next?.[key]);
      } catch {
        continue;
      }
      if (before !== after) changed.push(key);
    }
    return changed;
  }

  /**
   * 供应商相关的公开事件，比较前后两份设置里 Codex++ 下发的 activeRelayProfilePublic / modelCatalogFingerprint：
   *   - `relayProfile/switched`：({ previous, profile })，当前供应商换了
   *   - `modelCatalog/changed`：({ profile })，决定模型目录的字段变了（模型列表、每模型窗口 / 压缩 / 元数据、换供应商……），
   *     新目录用 /model-catalog/public 取
   * profile 形状同 /settings/public 的 activeProfile。页面里改的立即发，管理器里改的经心跳发；第一次读到设置时不发。
   */
  function emitCodexPlusRelaySettingsEvents(previous, next) {
    if (typeof codexPlusEmitExtensionEvent !== "function") return;
    const before = previous?.activeRelayProfilePublic;
    const after = next?.activeRelayProfilePublic;
    if (!before || !after) return;
    if (before.id !== after.id) {
      codexPlusEmitExtensionEvent("relayProfile/switched", [{ previous: { ...before }, profile: { ...after } }]);
    }
    const beforeFingerprint = previous?.modelCatalogFingerprint;
    const afterFingerprint = next?.modelCatalogFingerprint;
    if (beforeFingerprint && afterFingerprint && beforeFingerprint !== afterFingerprint) {
      codexPlusEmitExtensionEvent("modelCatalog/changed", [{ profile: { ...after } }]);
    }
  }

  /**
   * 在 Codex 页面里改了一项设置后发公开事件 `settings/changed`，参数 `{ key, value, persisted }`：
   * key 是改动的设置字段（存在 Codex++ 后端的用后端字段名，如 codexAppCapacityAutoRetry；只存页面本地的用页面键名）；
   * 先按新值发一次 persisted: false（这时读设置已是新值），写进后端后再发一次 persisted: true。
   * 跟着设置走的内置拓展订阅它，内核不再逐个点名功能。
   */
  function emitCodexPlusSettingsChanged(key, value, persisted) {
    if (typeof codexPlusEmitExtensionEvent === "function") codexPlusEmitExtensionEvent("settings/changed", [{ key, value, persisted: persisted === true }]);
  }

  /**
   * 启动期读取后端设置的重试间隔；返回 -1 表示停止。
   * 前 60 次每 250ms 一次（约 15 秒，覆盖常规启动）；之后按指数退避到 5 秒封顶再试 30 次，
   * 重启 Codex 后首次桥接超时、页面被启动壳自救重载这类慢启动也能补上设置与皮肤（#2680）。
   */
  function backendSettingsStartupRetryDelay(attempt) {
    if (attempt < 60) return 250;
    if (attempt >= 90) return -1;
    return Math.min(5000, 250 * 2 ** (attempt - 59));
  }

  function loadBackendSettingsForStartup(attempt = 0) {
    loadBackendSettings().then((loaded) => {
      if (loaded) {
        scan();
        return;
      }
      const delay = backendSettingsStartupRetryDelay(attempt);
      if (delay >= 0) {
        setTimeout(() => loadBackendSettingsForStartup(attempt + 1), delay);
      }
    });
  }

  let syncBackendSettingsInFlight = false;
  /** 已读到过的后端设置（对象引用，之后的写入都换新对象，不会改到它）；还没读到过时为 null。 */
  function codexPlusBackendSettingsSnapshot() {
    if (typeof codexPlusBackendSettingsLoaded === "undefined" || codexPlusBackendSettingsLoaded !== true) return null;
    return typeof codexPlusBackendSettings === "object" && codexPlusBackendSettings ? codexPlusBackendSettings : null;
  }

  async function syncBackendSettingsFromHeartbeat() {
    if (syncBackendSettingsInFlight) return false;
    syncBackendSettingsInFlight = true;
    try {
      // 记下读之前的设置：loadBackendSettingsState 换成新对象，旧引用里还是上一次的值。
      // 下面的 typeof 守卫：本段会被切片到只给了少量依赖的沙箱里测。
      const previous = codexPlusBackendSettingsSnapshot();
      const loaded = await loadBackendSettingsState();
      if (loaded) {
        syncOfficialUsagePolicy();
        const next = previous ? codexPlusBackendSettingsSnapshot() : null;
        if (previous?.modelCatalogFingerprint && next?.modelCatalogFingerprint
            && previous.modelCatalogFingerprint !== next.modelCatalogFingerprint) {
          await loadCodexModelCatalog(typeof refreshCodexModelCatalogForSettings !== "function");
        }
        // 启动期读取全部失败时，心跳是之后唯一的设置来源；跟着设置走的功能（含梦皮 #2680、对话宽度）由内置拓展订阅。
        emitCodexPlusSettingsLoaded("heartbeat", previous && next && typeof codexPlusChangedBackendSettingKeys === "function"
          ? codexPlusChangedBackendSettingKeys(previous, next)
          : undefined);
        if (previous && next && typeof emitCodexPlusRelaySettingsEvents === "function") emitCodexPlusRelaySettingsEvents(previous, next);
        // 管理器里改的设置：设置页开着时，开关与随总开关切换的说明也跟着变。
        syncCodexPlusBackendToggleButtons();
        renderCodexPlusMenu();
      }
      return loaded;
    } finally {
      syncBackendSettingsInFlight = false;
    }
  }

  async function setBackendSetting(key, value) {
    const seq = ++codexPlusBackendSettingsSeq;
    const previous = typeof codexPlusBackendSettingsSnapshot === "function" ? codexPlusBackendSettingsSnapshot() : null;
    codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
    codexPlusBackendSettingsLoaded = true;
    if (typeof emitCodexPlusSettingsChanged === "function") emitCodexPlusSettingsChanged(key, value, false);
    refreshCodexPlusBackendToggles();
    try {
      const settings = await postJson("/settings/set", { [key]: value });
      if (seq === codexPlusBackendSettingsSeq) {
        codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
        globalThis.window?.__codexPlusNativeFastUnlock?.runtime?.update?.({
          enhancementsEnabled: codexPlusBackendSettings.enhancementsEnabled !== false,
          codexAppNativeFastUnlock: codexPlusBackendSettings.codexAppNativeFastUnlock === true,
          nativeFastLocalApiConfigured: codexPlusBackendSettings.nativeFastLocalApiConfigured === true,
        });
        globalThis.window?.__codexPlusNativePredictions?.runtime?.update?.({
          enhancementsEnabled: codexPlusBackendSettings.enhancementsEnabled !== false,
          codexAppNativePredictions: codexPlusBackendSettings.codexAppNativePredictions === true,
          nativePredictionsLocalApiConfigured: codexPlusBackendSettings.nativePredictionsLocalApiConfigured === true,
        });
        if (typeof emitCodexPlusSettingsChanged === "function") emitCodexPlusSettingsChanged(key, value, true);
        if (previous && typeof emitCodexPlusRelaySettingsEvents === "function") emitCodexPlusRelaySettingsEvents(previous, codexPlusBackendSettings);
      }
    } finally {
      refreshCodexPlusBackendToggles();
    }
  }

  /**
   * 把直接写后端字段的开关同步成当前值。心跳每 5 秒也会调用，
   * 值没变就不写属性，避免无谓的 DOM 变更触发扫描。
   */
  function syncCodexPlusBackendToggleButtons() {
    document.querySelectorAll("[data-codex-plus-usage-policy]").forEach((input) => {
      const saving = input.dataset.saving === "true";
      const disabled = codexPlusBackendSettings.usageHistoryEnabled !== true || saving;
      if (input.disabled !== disabled) input.disabled = disabled;
      const key = input.dataset.codexPlusUsagePolicy;
      const unit = key === "usageHistoryStorageMiB" ? input.nextElementSibling : null;
      if (unit && unit.disabled !== disabled) unit.disabled = disabled;
      const stored = codexPlusBackendSettings[key];
      const value = unit ? (stored ?? 2048) / Number(unit.value) : stored;
      if (value !== undefined && document.activeElement !== input && !saving && input.value !== String(value)) input.value = String(value);
    });
    document.querySelectorAll(".codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-backend-setting");
      const enabled = String(!!codexPlusBackendSettings[key]);
      if (button.dataset.enabled === enabled) return;
      button.dataset.enabled = enabled;
      button.setAttribute("aria-checked", enabled);
    });
    syncCodexPlusSettingsCatalogDescriptions();
  }

  /** 刷新设置页的开关与菜单。跟着设置走的功能由内置拓展订阅 settings/loaded、settings/changed 自己对齐，这里不点名。 */
  function refreshCodexPlusBackendToggles() {
    syncCodexPlusBackendToggleButtons();
    renderCodexPlusMenu();
    scan();
  }

  let codexPlusUserScripts = { enabled: true, builtin_dir: "", user_dir: "", scripts: [] };
  // 单独跟踪「读过了没有」：scripts 为空既可能是真没有脚本，也可能是还没读到。
  // 不区分就会在无后端时把「正在读取」直接显示成「未发现」。
  let codexPlusUserScriptsLoaded = false;
  // 市场清单。为空 + 未加载 = 还在拉；加载过为空 = 市场里确实没东西。
  // packages：精选 / 官方拓展包（锁定到审核过的提交），和管理器「精选」栏同一份清单。
  let codexPlusScriptMarket = { scripts: [], packages: [], loaded: false, loading: false, message: "" };
  // 正在准备 / 安装的拓展包 id（按钮转圈、防重复点击）。
  let codexPlusPackageInstalling = "";
  // 「拓展」页左面板的搜索关键词，纯前端过滤。
  let codexPlusExtensionsQuery = "";
  let codexPlusExtensionsFilter = "market";
  // 当前选中的拓展（左面板点开后右侧显示详情）。空 = 还没选。
  let codexPlusExtensionsSelected = null;
  // 默认扩展图标：VSCode codicon 的 `extensions` 字形（\eae6），
  // 从本机 VSCode 的 codicon.ttf 抽出轮廓后归一到 16x16 视口。
  // 市场清单目前没有图标字段，所有市场条目都用它；本地脚本同理。
  // Codex++ 的 logo（apps/codex-plus-manager/src/assets/codex-plus-logo.png 缩到 48px），图标栏入口用。
  const codexPlusLogoDataUrl = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAYAAABXAvmHAAAABGdBTUEAALGPC/xhBQAAACBjSFJNAAB6JgAAgIQAAPoAAACA6AAAdTAAAOpgAAA6mAAAF3CculE8AAAARGVYSWZNTQAqAAAACAABh2kABAAAAAEAAAAaAAAAAAADoAEAAwAAAAEAAQAAoAIABAAAAAEAAAAwoAMABAAAAAEAAAAwAAAAANs3bAwAAAHJaVRYdFhNTDpjb20uYWRvYmUueG1wAAAAAAA8eDp4bXBtZXRhIHhtbG5zOng9ImFkb2JlOm5zOm1ldGEvIiB4OnhtcHRrPSJYTVAgQ29yZSA2LjAuMCI+CiAgIDxyZGY6UkRGIHhtbG5zOnJkZj0iaHR0cDovL3d3dy53My5vcmcvMTk5OS8wMi8yMi1yZGYtc3ludGF4LW5zIyI+CiAgICAgIDxyZGY6RGVzY3JpcHRpb24gcmRmOmFib3V0PSIiCiAgICAgICAgICAgIHhtbG5zOmV4aWY9Imh0dHA6Ly9ucy5hZG9iZS5jb20vZXhpZi8xLjAvIj4KICAgICAgICAgPGV4aWY6Q29sb3JTcGFjZT4xPC9leGlmOkNvbG9yU3BhY2U+CiAgICAgICAgIDxleGlmOlBpeGVsWERpbWVuc2lvbj42NDwvZXhpZjpQaXhlbFhEaW1lbnNpb24+CiAgICAgICAgIDxleGlmOlBpeGVsWURpbWVuc2lvbj42NDwvZXhpZjpQaXhlbFlEaW1lbnNpb24+CiAgICAgIDwvcmRmOkRlc2NyaXB0aW9uPgogICA8L3JkZjpSREY+CjwveDp4bXBtZXRhPgohBDnEAAAO70lEQVRoBe1ZfYxc1X099703M7uz37tmbWNwvpqQYEgIBgIJCdAQUoJCSgIViSpS1YWglv+qNgpSIqq2EYRISQQoQlEai5oSiNoGUKzWpkCBJDSJFejiYIxtdm3vendnd3Zndr7nvXt7zp2ZYHttszUmUqVe7Xtv3nv3/u45v/v7um+B/+PN/K7xu69/fATp5gY07LS5/bndb3b+3xkBd9e1fYgWb3BJcotJkvcjsTM2dk87434cZoefMH+1rXwyZN5yAvk7rxwYCmqfcc7eaqzdCGfTiC1gHOB02DIS7ERottSDYGtmFPvNl3Y0V0rmLSPg7r6qB7Z0tXPuZiTxJ4yzxsUOhEzwrZMRAT5pXS25YNKE0aN89q+FTM+Owa88t6CeJ2qnnEALePF6WHsTbHIZUYWuSXACawzhBjBhCBMYXgErNs2ER5Nv2Cdkn9hVCXonu29FmH3Y3PFfvzkeiVNGIH/nxoEhE15BDH/pbHIRNZ4WMCslE4k3F15NIAIBGuU6KvkK+lYPwWQzQEwScQNokIiliZFIa0wwA2OfSSrxD57dsXv7FU8jPpzMKSHQuOP8i1P93V9F3LyKR+QI3DkBkK2LAEHzanhqVhrITxUxtTcPu1TBhvVppEdHgNNWAdkudiLjep2YEw51SBoJSvkyDk6Ux1854D7+ud2FfYcTiA6/ObnfRLp3zW3ojT7levvhhvqBNIE36m0CAQKaS1xtIDe+gEP78igVG4g5LBXQcvg8k5uFzeeBTDcwNAhz2hCDlMPs3hwm9pewVIwZdU2qHkXscGQ7BQSAoJkYzBaRHMrDZboQSKPD/TBRhKRcwczELI9FlItNbxUhTYgKhu0bQdKf4o9ZICHhGolPVYC5eZAr9kzVkJBomv0zqcDVTfpI9Lw7JQSIiutNM6FLukoFdk8J6EojGRzCPiLJ5wiKb0PavxpholmL0XPFFUjfchvszieBX/wEmHwFLqFbx9R4mSIpU6ukFWSsIvzlBFoSvdiTPhkLOmw9YbRUpCGRFPVCp6zvn0ZhroKIGozkwIxEjuASHgo8JoiQeu9GBJ//G+CvHwRG38Y+0oUB/3xUCjSO0HRoDY5GedIr4J66PMLz9lzgkj91xdWfsjnaMKOKqytISF/tiBOQlP4IPggjdL3nHFTm8qjPH0Czwaij6JNuwPQNw6VbJu6EnuM9ai+NJ0GnnKPbSRBg8v/GJR+0v6h+kdr6vDHJachQDMNhNMDctVjmUSORBu28Hf+pSxm9DUKcfuvtyKx5ByZ+tBm2Z0jMWsjkFD5XkDwJcCF9450yB7E7NLA8Qa+YADVo4rsv+lgQX3ALjfhKJp1Rac8q0BOo1046jWAVo85ANxyjS5QnkUIZ5XKMiF2ikKsQpTF0zoU8zmNkKvNe2YwvpfFEeYAH0YuLkpzSQcDQyh7H8IAVOLH7weVdmK9eYO+68OYwsdfRUfsY0zxwJ/CdldYSa+mlbRIx6RSyXJEPrBnE+L4lzE4UUG2Pk2rl9yH7OIJn4qPTZ+Gu3ASzfTPcgZdhmeiSJEUu1LGmEcFjNNI/dnOP3BDesbHvI6g0b6fX/R3N5UOmmWSsJpTGify3ZnqEcDFSUSDnYwjMpDC6rhcjp/dDvj0UVpHu7wUGR+nsBO1i9pI49n7fh2EuvhZY9XYUSTasF5jUqn4l2KHUbJot/zTXYMx9vS2jRVsz+O7VV6Gy+OeI48u4kANQLZMwuyYtjfvhHnRnOJ9To94UxM3bk57xvQgTokxF0clVq7yPUB98Fxof+TP0XnI1Iw8dn1lbYRN0dJgMkkoRwf234KWHHkGu3k1TMocqdXzyml3lsdfh+/U+/JYm+MW1Z+O18fuZQa5FkBqwnMy2NerTvPTFe9UzAYO0STERpVnLcGJDJ2XQbmnT67VFmHURbJPhs85EFmU4JoSdfAHFJzbD5g4y4fGZFMIizxmZTA1Rhv5EuZTgp6USjA2VFY5syx6YuNHn9u4fxI6dSF47BFeidkImEKV5lY9sgSpG1ii5PfNYZLo3VD4yNIcU+4kAfUHKb83OH/INIaHPKA84On92cBBnmH2IHv0yzAuPwlbKaD7+XZixbQyt7MO5tOpqGto5+5+HnUj3qBakOJY2IyGTJDA5DfT2sj4ZRsCDqkThQA77d80hN1vB2n6DwdIcayCGxFHadU8fl5HVMCtLAVBt5s1DpBhafO0vbbdNrfSrZzG57ZdY97UtcI89gOb0HmQuvwrRpdcDlSUmL0UgBdEgCOjTR6E9TimhdbE8RXQshbjSElyRxzRrmlrEqLLgi60m1VxrcIIa65jZHGsY1kIkYtashusm6RTjdr1GMppWnqDVay16tVjHwb2LOHhwCc3MINYwfJpUN6q5EuKt/4LKM0+gUDUoum7FNQ7X+NQKCWgq/VF7ztsmidBsDMHUcq2CTCne0e6TQRZkdp6imbjofC7H1VBWVnnMos509xA1CSbasJBwoYrZySVMHiihVKFyCCzqobkQY2xZbnNe1zCY5vvJRgopX4bwpXDQ3Y5uy02IziahQu91JhJ+lOITIUgQnyicJlEfhv/i64jWk8R/PAD85hlqvETT4YgZlsizc978gjPX0gxCHNo1RY2XUKnQtiknpAIs+yoPJI0aGqyllMe85dPkaADehNRXC5A0yOyotpyAgjW3d1RniwjB+mxICV6wCPBPGEUxNTSC4KJPAxv/AO7Fp+C+fRMj9iLNjyFTobdQgCuXMbHgMD7f5FACV/ZtK0blRVCvIHjsTmQK46hSfszpVaErE7fMh6hZU6Wi5Uvg3x9Byq8A3YaApXnhnV2ymKhwUsX0apMVBEsICvfm5XvxTia27vd8NJKHOt4HDJcBs61WLWb5qWtIzVKkH+tX2Ido4pt4CSPZCtad2cU9TUT+6icM7MUwzNn5NYaF71FtOQEJJ/LWlwKvbEznG3jevRuNL92L6PdvQNA7jJibj1grJZadprCnwZxaID17aYDmqHpGfaUYVSDqplJEBZ+/idmPq96VTcF2RahSgkwozSNGUKgm+N68DV7tTNW5HtOEDA3Ra8eDoDVwtnQ3o8Gln8GZn/gChvfswsEfP4zZl19Cz7ozCIB+4/2jDZoXv36MUsSph/7QnVaudcc3JNemirjSRKluMV6wmFqwyFC1oXG1ehI+XrX41md3lZ8Hyi1xXmbrtJwATUhakla8M3pBrLV4NZZ1CzXZ854NeO+X/x7vlu3Snm3MTXiqixqlNpU/NN4rtjWftK+F0PoLvG68gvhayVt34/MJitwUVZu09cAkDNHba3F4z6dfrvwbR2joMdtyAuom/O3uuno+EqGMzGyikGgdw6LKCFWSHhY7MGS6kfVAboqDmMzkyKwm5SvSvExHSbnFRKR4Q+F6lK8ySPB9lnaTmOjFUpK67bqXC3v56oTtmD4g+L/VFH909aWwOplC8Ow/ws1IJofxA0HL0QWEjxQ6Bpitv/IQzM3fhj37owTM+M5q0tu9yghB4UkK6TQNlb8p0kmbyhWMUsXBVUPznT4nunIBj2xfPSd7RuCSP+ak3VK6Pu309Xfh9LUZhDMvwr36n9x876TJRwiGV3NVuArqxIm9xWd7EagsvvQ6mLMuYQafR8DyYL5sUOCW1mvdoxaXtgfoXo2aiLhE/IKy3ybRjx6YXNIXuhO245iQ1MQ/bzv8wVIhPjQHU6AjU2a4MIN47ElM956P0269C139/B6kCKRGk3JJCUFXF8yHroEpzsD++t/5gqbkxYqEZLfNyYPXqRWdZGbezlhSraQtIyBLaD2UJM+jdVW4KyzB8nC93XhpqoGJoIIrb+InE1aWflclZCFLY42jYyNQEUTT4VAB9i+8NGFsEfErRyfnt1/fQf7C/qa66OvZdu/jX5b7gLajsldO2JqYgvnb5wWWyo5HUqqiyP1uQPPxJsEO2guYGsk8+T1+kfoZn/NeqlDy4/yC0/YsD1RGZ9thllmCctlBCmixdN2Mobx5w7ZsBfhxlapxXG9OSBaeCE8SzTve63XHDNpzCB19wi7Mo7nlmwgH0oguuhq4fBNHaaRktQ8PkvI0xmNUhGr/7PSltJo+ya2gLSPQiE2OaXxPyrgPKgvWhZGTWiGQgWo2z04/2cEHcj1kPOD7ZqGO5v59CA/sR/qFp5hK+/nq9Vih9Qja2LR63uYp25sOpaT5rMR/elRNF435jdsyExr8SWFfoZm9cbYe/W0lDiaUwLRF9/iJU0TESS5huG/F/EHecUtJMr7cIye9j5caqP33GGq//jmjCsOpuPPQR9uYN3L5mL91r0OrrXkmm27stYr9xh/t2Efhb9xeV81hfe9+tTL/zT2Np64Z7drOEkxl+Nu7A9cjtgIuElF/Fu9Yl8Lgwhgnr8EMr+N3UX4K2fYgt7ScW/tbgVa5TI3P0z0W6NcyRhH0J/+DQYM1fyV2S69W7SNbppOv3TdVe/YwOCf8uSI7++eLB85/f3dj00gq+WyvcWsSIou56U6vYSm9agBBN5159TuRDLwL1Yc3wy1xU9Pe3CsQCPJu7nEmiqxQOaN3Az7V6soMZxOMPVeI7/mHmURlA/ewx/gEdxwaKyLQGbvlvOyFZ/UmN56ZxvVDUbK+Qe1afvcJVw0hs5ZbSVa7lbFxVhoq7lq2pFWQaeyaZ72zFMhTPIk0mbBKn95VdQ/+cK75yO4qdvPVEo92QunMeuLr/4pAW1T0nXOzH/jYoNt0eir+XG9oRxV2E34CCXv4PYf/D7D6LySbd3IS4X8D8ErOYaLED118zpIwOZTgZ9sX7b2P55Pn+EhlA/diLevidcXtZAh0hIf3nt19waUj7k/Wpu0f9gcyLYu613enS8dcHPiPGUySwHxsD75SNQ/dOxX/kG7xGntK68p4J9XeDIHOhOb752Q3ntuf3Lgu424Yitx6VsT6mEd1trw0w3g/ljeNn85h69YC7n+6GP+Kg/lfEPI9Ca13Jtb1VBDoyAvv29Bz7sWD8aa1KXd9NrBr5LDaqO8tm52Pztjv33fAPsavnZMc8KaBdyZ9K67mnrOyF7z44cx3Dl2WGtt2XmrzRwdS53Mifl85pQp7K7AfKfNb78uuBTbIb/+/HU8D/wMq1TmUmouq+wAAAABJRU5ErkJggg==";
  const codexPlusDefaultExtensionIconPath = "M15.0 4.95 Q15.0 4.37 14.63 3.99 L12.01 1.37 Q11.63 1.0 11.05 1.0 Q10.46 1.0 10.08 1.37 L8.0 3.46 L8.0 3.3 Q8.0 2.71 7.6 2.31 Q7.2 1.91 6.61 1.91 L2.39 1.91 Q1.8 1.91 1.4 2.31 Q1.0 2.71 1.0 3.3 L1.0 13.61 Q1.0 14.2 1.4 14.6 Q1.8 15.0 2.39 15.0 L12.7 15.0 Q13.24 15.0 13.66 14.6 Q14.09 14.2 14.09 13.61 L14.09 9.39 Q14.09 8.8 13.66 8.4 Q13.24 8.0 12.7 8.0 L12.54 8.0 L14.63 5.92 Q15.0 5.54 15.0 4.95 Z M2.39 2.87 L6.61 2.87 Q6.77 2.87 6.93 3.0 Q7.09 3.14 7.09 3.3 L7.09 8.0 L1.91 8.0 L1.91 3.3 Q1.91 3.14 2.04 3.0 Q2.18 2.87 2.39 2.87 Z M1.91 13.61 L1.91 8.91 L7.09 8.91 L7.09 14.09 L2.39 14.09 Q2.18 14.09 2.04 13.96 Q1.91 13.82 1.91 13.61 Z M13.13 9.39 L13.13 13.61 Q13.13 13.82 13.0 13.96 Q12.86 14.09 12.7 14.09 L8.0 14.09 L8.0 8.91 L12.7 8.91 Q12.86 8.91 13.0 9.07 Q13.13 9.23 13.13 9.39 Z M8.0 8.0 L8.0 6.45 L9.55 8.0 Z M13.93 5.27 L11.37 7.84 Q11.21 8.0 11.02 8.0 Q10.83 8.0 10.73 7.84 L8.11 5.27 Q8.0 5.11 8.0 4.93 Q8.0 4.74 8.11 4.63 L10.73 2.02 Q10.83 1.91 11.02 1.91 Q11.21 1.91 11.37 2.02 L13.93 4.63 Q14.09 4.74 14.09 4.93 Q14.09 5.11 13.93 5.27 Z";
  let codexPlusBackendStatus = window.__codexPlusBackendStatus || { status: "checking", message: "正在检查后端…" };
  let codexPlusBackendCheckSeq = 0;
  let codexPlusBackendCheckInFlight = false;
  let codexPlusBackendFailureCount = 0;
  /** 上次心跳成功读取的设置版本。 */
  let codexPlusBackendSettingsRevision = "";
  let codexPlusWeixinSyncRevision = null;
  let codexPlusWeixinSyncHeartbeat = null;
  const CODEX_PLUS_BACKEND_FAILURE_THRESHOLD = 3;
  // 桥接通道（binding）与后端可用性分开统计：HTTP 回落成功会让后端状态保持绿色，
  // 但桥接持续失败时必须把降级呈现出来，否则启动器侧的重注入修复循环对用户完全不可见（issue #2169）。
  let codexPlusBridgeFailureCount = 0;
  const CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD = 3;
  const codexPlusBackendGeneration = (Number(window.__codexPlusBackendGeneration) || 0) + 1;
  window.__codexPlusBackendGeneration = codexPlusBackendGeneration;

  function recordCodexPlusBridgeHealth(field) {
    if (codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
    const health = window.__codexPlusBridgeHealth || (window.__codexPlusBridgeHealth = {});
    health[field] = Date.now();
  }

  function recordCodexPlusBridgeSuccess() {
    recordCodexPlusBridgeHealth("lastSuccessAt");
    if (codexPlusBackendGeneration === window.__codexPlusBackendGeneration) {
      const health = window.__codexPlusBridgeHealth || (window.__codexPlusBridgeHealth = {});
      health.failures = 0;
    }
    codexPlusBridgeFailureCount = 0;
  }

  function recordCodexPlusBridgeAttempt() {
    recordCodexPlusBridgeHealth("lastAttemptAt");
  }

  function recordCodexPlusBridgeFailure() {
    codexPlusBridgeFailureCount += 1;
    if (codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
    // 看门狗只能读到 window 上的 health 快照：不落 failures/lastFailureAt，
    // CDP 连接断掉后它会一直把“还在尝试”当成桥接存活，永久不重注入。
    const health = window.__codexPlusBridgeHealth || (window.__codexPlusBridgeHealth = {});
    health.lastFailureAt = Date.now();
    health.failures = codexPlusBridgeFailureCount;
  }

  function renderBackendStatus() {
    const bridgeDegraded = codexPlusBridgeFailureCount >= CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD;
    const rawStatus = codexPlusBackendStatus.status || "failed";
    const status = bridgeDegraded && rawStatus === "ok" ? "degraded" : rawStatus;
    if (codexPlusBackendStatus.version) {
      codexPlusVersion = codexPlusBackendStatus.version;
      document.querySelectorAll("[data-codex-plus-version]").forEach((node) => {
        node.textContent = `Codex++ ${codexPlusVersion}`;
      });
    }
    const labelFallback = status === "ok" ? "后端已连接" : status === "degraded" ? "桥接降级，自动修复中" : status === "checking" ? "正在检查后端…" : "未连接";
    const label = document.querySelector("[data-codex-backend-status]");
    if (label) {
      label.dataset.status = status;
      label.textContent = status === "degraded" ? labelFallback : (codexPlusBackendStatus.message || labelFallback);
    }
    document.querySelectorAll("[data-codex-backend-indicator]").forEach((indicator) => {
      indicator.dataset.status = status;
      indicator.title = status === "ok" ? "后端已连接" : status === "degraded" ? "后端可达，桥接降级，正在自动修复" : status === "checking" ? "正在检查后端" : "未连接";
    });
    const sidebarStatus = document.querySelector(`#${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status`);
    if (sidebarStatus) {
      sidebarStatus.dataset.status = status;
      sidebarStatus.title = status === "ok" ? "后端已连接" : status === "degraded" ? "后端可达，桥接降级，正在自动修复" : status === "checking" ? "正在检查后端" : "未连接";
    }
  }

  function withBackendTimeout(request) {
    return Promise.race([
      request,
      new Promise((resolve) => setTimeout(() => resolve({ status: "failed", message: "后端检查超时", timeout: true }), 2000)),
    ]);
  }

  async function checkBackendStatus() {
    if (codexPlusBackendCheckInFlight) return;
    codexPlusBackendCheckInFlight = true;
    const seq = ++codexPlusBackendCheckSeq;
    try {
      const nextStatus = await postJson("/backend/status", {});
      if (seq !== codexPlusBackendCheckSeq || codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
      if (nextStatus?.status === "ok") {
        codexPlusBackendFailureCount = 0;
        codexPlusBackendStatus = window.__codexPlusBackendStatus = nextStatus;
        if (typeof nextStatus.hideOfficialUsageAlert === "boolean") {
          window.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = nextStatus.hideOfficialUsageAlert;
          syncOfficialUsagePolicy();
        }
        // 设置没变就不拉 /settings/get：那份 JSON 有 166 KB，五秒一次的编解码本身就够把
        // 渲染进程主线程切碎。后端在 status 里带上快照版本，这里只比对字符串。
        // 老后端不带版本（空串）时退回原来的每次全量拉取，行为不变。
        const revision = typeof nextStatus.settingsRevision === "string" ? nextStatus.settingsRevision : "";
        const weixinRevision = nextStatus.weixinDesktopSyncRevision ?? null;
        const settingsStale = !revision
          || !codexPlusBackendSettingsLoaded
          || revision !== codexPlusBackendSettingsRevision
          || weixinRevision !== codexPlusWeixinSyncRevision;
        if (settingsStale) {
          void syncBackendSettingsFromHeartbeat().then((loaded) => {
            if (loaded && codexPlusBackendGeneration === window.__codexPlusBackendGeneration) {
              codexPlusBackendSettingsRevision = revision;
              codexPlusWeixinSyncRevision = weixinRevision;
            }
          }).catch(() => {});
        } else {
          codexPlusWeixinSyncHeartbeat?.();
        }
      } else {
        codexPlusBackendFailureCount += 1;
        sendCodexPlusDiagnostic("backend_check_failed", {
          status: nextStatus?.status || "unknown",
          message: nextStatus?.message || "",
          timeout: !!nextStatus?.timeout,
          consecutiveFailures: codexPlusBackendFailureCount,
        });
        if (codexPlusBackendFailureCount >= CODEX_PLUS_BACKEND_FAILURE_THRESHOLD) {
          codexPlusBackendStatus = window.__codexPlusBackendStatus = nextStatus;
        }
      }
      renderBackendStatus();
    } finally {
      codexPlusBackendCheckInFlight = false;
    }
  }

  async function openManagerFromCodex() {
    const result = await postJson("/manager/open", {});
    if (result.status === "ok") {
      showToast("管理工具已打开", null);
    } else {
      showToast(result.message || "打开管理工具失败", null);
    }
  }

  function scheduleBackendHeartbeat() {
    if (codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
    if (window.__codexPlusBackendHeartbeat &&
        window.__codexPlusBackendHeartbeatGeneration === codexPlusBackendGeneration) return;
    if (window.__codexPlusBackendHeartbeat) clearInterval(window.__codexPlusBackendHeartbeat);
    window.__codexPlusBackendHeartbeatGeneration = codexPlusBackendGeneration;
    window.__codexPlusBackendHeartbeat = setInterval(checkBackendStatus, 5000);
    checkBackendStatus();
  }

  function userScriptStatusLabel(status) {
    return { loaded: "已加载", failed: "失败", disabled: "已禁用", not_loaded: "未加载", loading: "加载中", unknown: "状态待确认" }[status] || status || "未知";
  }

  // 使用已安装 Lucide Search / RotateCw / Plus / ArrowLeft / Settings 的 ISC 图标路径。
  const codexPlusDiscoveryIcons = {
    search: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>',
    refresh: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/></svg>',
    plus: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M12 5v14"/></svg>',
    back: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 19-7-7 7-7M19 12H5"/></svg>',
    settings: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>',
  };

  /** 原生风格侧栏：浏览入口和已安装拓展的单行名称。 */
  function renderCodexPlusExtensionsNav() {
    const { installed } = codexPlusExtensionsEntries();
    const shown = filterCodexPlusExtensionsEntries(installed);
    const rows = shown.map((entry) => {
      const selected = codexPlusExtensionsSelected?.kind === "installed" && codexPlusExtensionsSelected.key === entry.key;
      const marketItem = codexPlusExtensionMarketItem(entry);
      return `
        <button type="button" class="codex-plus-page-nav-item codex-plus-extensions-nav-entry" data-codex-extensions-select="installed:${escapeHtml(entry.key)}" data-active="${String(selected)}" title="${escapeHtml(entry.name)}" aria-label="${escapeHtml(entry.name)}">
          <span class="codex-plus-page-nav-item-icon" aria-hidden="true">${extensionIconMarkup(codexPlusExtensionIcon(marketItem, entry.script))}</span>
          <span class="codex-plus-page-nav-item-text"><span class="codex-plus-page-nav-item-name">${escapeHtml(entry.name)}</span></span>
          <span class="codex-plus-page-nav-item-state" data-state="${entry.enabled ? "on" : "off"}" title="${entry.enabled ? "已启用" : "已禁用"}"></span>
        </button>
      `;
    }).join("");
    const empty = codexPlusExtensionsQuery.trim() ? "没有匹配的已安装拓展。" : codexPlusUserScriptsLoaded ? "未发现已安装的拓展。" : "正在读取用户拓展…";
    return `
      <button type="button" class="codex-plus-page-nav-item codex-plus-extensions-nav-entry" data-codex-extensions-browse="market" data-active="${String(!codexPlusExtensionsSelected && codexPlusExtensionsFilter === "market")}" title="浏览拓展" aria-label="浏览拓展">
        <span class="codex-plus-page-nav-item-icon" aria-hidden="true">${extensionIconSvg()}</span>
        <span class="codex-plus-page-nav-item-text"><span class="codex-plus-page-nav-item-name">浏览拓展</span></span>
      </button>
      <div class="codex-plus-page-nav-group">
        <div class="codex-plus-page-nav-group-head"><span>已安装</span><span class="codex-plus-page-nav-group-count">${installed.length}</span></div>
        ${rows || `<div class="codex-plus-page-nav-empty">${escapeHtml(empty)}</div>`}
      </div>
    `;
  }

  /** 拓展页的分栏：和管理器「拓展」页同一排（精选 / 社区 / 本地 / 我发布的 / dsh 插件）。 */
  const codexPlusExtensionsFilters = [
    ["market", "精选"],
    ["community", "社区"],
    ["installed", "本地"],
    ["mine", "我发布的"],
    ["dsh", "dsh 插件"],
  ];

  function codexPlusExtensionsFilterValue(value) {
    return codexPlusExtensionsFilters.some(([id]) => id === value) ? value : "market";
  }

  /** 分栏胶囊 + 右侧排序（精选、社区才有）。 */
  function renderCodexPlusExtensionsFilterBar() {
    const counts = {
      market: codexPlusScriptMarket.loaded ? (codexPlusScriptMarket.scripts || []).length + (codexPlusScriptMarket.packages || []).length : null,
      installed: codexPlusUserScriptsLoaded ? (codexPlusUserScripts.scripts || []).length : null,
    };
    const pills = codexPlusExtensionsFilters.map(([id, label]) => {
      const active = codexPlusExtensionsFilter === id;
      const count = counts[id] == null ? "" : ` <span class="codex-plus-extensions-filter-count">${counts[id]}</span>`;
      return `<button type="button" data-codex-extensions-browse="${id}" data-active="${String(active)}" aria-pressed="${String(active)}">${escapeHtml(label)}${count}</button>`;
    }).join("");
    const sort = typeof renderCodexPlusExtensionsSortControl === "function" ? renderCodexPlusExtensionsSortControl(codexPlusExtensionsFilter) : "";
    return `<div class="codex-plus-extensions-filterbar"><div class="codex-plus-extensions-filter" role="group" aria-label="拓展来源">${pills}</div>${sort}</div>`;
  }

  /** 列表里的一节：标题带数量，下面是两列卡片。 */
  function renderCodexPlusExtensionsSection(title, cards) {
    return `
      <h2 class="codex-plus-discovery-section-title">${escapeHtml(title)} <span class="codex-plus-discovery-section-count">${cards.length}</span></h2>
      <div class="codex-plus-extensions-grid" data-codex-plus-extensions-list="true">${cards.join("")}</div>
    `;
  }

  /** 状态小标签（已安装 / 可更新 / 本机源码 …），tone：neutral / ok / warn / danger。 */
  function codexPlusExtensionBadge(text, tone = "neutral", title = "") {
    return `<span class="codex-plus-extension-badge" data-tone="${tone}"${title ? ` title="${escapeHtml(title)}"` : ""}>${escapeHtml(text)}</span>`;
  }

  /** 精选里已安装条目的状态：与管理器一致，只给已安装 / 可更新 / 本机源码打标。 */
  function codexPlusCuratedStatusBadge(item) {
    if (!item) return "";
    if (item.localSource) return codexPlusExtensionBadge(`本机源码 ${item.installedVersion || ""}`.trim(), "neutral", "你在本机编写并发布的拓展，在「本地」里管理");
    if (item.updateAvailable) return codexPlusExtensionBadge("可更新", "warn");
    const version = item.installedVersion || String(item.installedSha || "").slice(0, 7);
    return codexPlusExtensionBadge(version ? `已安装 ${version}` : "已安装");
  }

  /** 一张拓展卡片：左边选中按钮（图标、名字、标签、简介），右边独立的操作列，避免嵌套按钮。 */
  function codexPlusExtensionCardHtml(entry, { status = false } = {}) {
    const item = codexPlusExtensionMarketItem(entry);
    const description = item?.description || entry.script?.description || entry.meta || "";
    const select = `${entry.kind}:${escapeHtml(entry.key)}`;
    const isPackage = item?.kind === "package";
    const installAttr = isPackage ? "data-codex-package-install" : "data-codex-market-install";
    const installing = isPackage && codexPlusPackageInstalling === item.id;
    const busyAttrs = `${codexPlusPackageInstalling && isPackage ? " disabled" : ""}${installing ? ' data-busy="true"' : ""}`;
    const actions = [];
    if (entry.kind === "market") {
      actions.push(`<button type="button" class="codex-plus-discovery-icon-button" ${installAttr}="${escapeHtml(entry.key)}" aria-label="安装 ${escapeHtml(entry.name)}" title="${installing ? "正在准备安装…" : `安装 ${escapeHtml(entry.name)}`}"${busyAttrs}>${codexPlusDiscoveryIcons.plus}</button>`);
    } else {
      // 精选里已安装、有审核过的新版本：给「更新」，走同一条安装流程。
      if (status && item?.updateAvailable) {
        actions.push(`<button type="button" class="codex-plus-discovery-icon-button" ${installAttr}="${escapeHtml(item.id)}" aria-label="更新 ${escapeHtml(entry.name)}" title="更新 ${escapeHtml(entry.name)}"${busyAttrs}>${codexPlusDiscoveryIcons.refresh}</button>`);
      }
      actions.push(`<button type="button" class="codex-plus-toggle" data-codex-user-script-key="${escapeHtml(entry.key)}" data-enabled="${String(entry.enabled)}" role="switch" aria-checked="${String(entry.enabled)}" aria-label="启用 ${escapeHtml(entry.name)}"><span></span></button>`);
    }
    const badges = `${codexPlusOfficialBadge(item)}${status ? codexPlusCuratedStatusBadge(item) : ""}`;
    return `
        <article class="codex-plus-extension-card">
          <button type="button" class="codex-plus-extension-card-select" data-codex-extensions-select="${select}" aria-label="查看 ${escapeHtml(entry.name)}">
            <span class="codex-plus-extension-card-icon" aria-hidden="true">${extensionIconMarkup(codexPlusExtensionIcon(item, entry.script))}</span>
            <span class="codex-plus-extension-card-copy">
              <span class="codex-plus-extension-card-heading"><span class="codex-plus-extension-card-title">${escapeHtml(entry.name)}</span>${badges}</span>
              <span class="codex-plus-extension-card-description">${escapeHtml(description)}</span>
            </span>
          </button>
          <div class="codex-plus-extension-card-actions">${actions.join("")}</div>
        </article>
      `;
  }

  /**
   * 主区：分栏条 + 当前分栏的列表。精选分「已安装 / 可安装」，本地分「已启用 / 未启用」（与管理器一致）；
   * 社区、我发布的、dsh 插件由 41-extension-sources.js 画。
   */
  function renderCodexPlusExtensionsOverview() {
    const filter = codexPlusExtensionsFilterValue(codexPlusExtensionsFilter);
    const head = renderCodexPlusExtensionsFilterBar();
    if (filter === "community" || filter === "mine" || filter === "dsh") {
      return `${head}${typeof renderCodexPlusExtensionSourcePane === "function" ? renderCodexPlusExtensionSourcePane(filter) : ""}`;
    }
    const { installed, market, curated } = codexPlusExtensionsEntries();
    const query = codexPlusExtensionsQuery.trim();
    let sections;
    let empty;
    if (filter === "installed") {
      const entries = filterCodexPlusExtensionsEntries(installed);
      const on = entries.filter((entry) => entry.enabled).map((entry) => codexPlusExtensionCardHtml(entry));
      const off = entries.filter((entry) => !entry.enabled).map((entry) => codexPlusExtensionCardHtml(entry));
      sections = `${on.length ? renderCodexPlusExtensionsSection("已启用", on) : ""}${off.length ? renderCodexPlusExtensionsSection("未启用", off) : ""}`;
      empty = query ? `没有匹配「${query}」的拓展。` : codexPlusUserScriptsLoaded ? "未发现已安装的拓展。" : "正在读取用户拓展…";
    } else {
      const own = filterCodexPlusExtensionsEntries(curated).map((entry) => codexPlusExtensionCardHtml(entry, { status: true }));
      const available = filterCodexPlusExtensionsEntries(market).map((entry) => codexPlusExtensionCardHtml(entry));
      sections = `${own.length ? renderCodexPlusExtensionsSection("已安装", own) : ""}${available.length ? renderCodexPlusExtensionsSection("可安装", available) : ""}`;
      empty = query ? `没有匹配「${query}」的拓展。`
        : codexPlusScriptMarket.loading && !codexPlusScriptMarket.loaded ? "正在读取拓展…"
        : codexPlusScriptMarket.message || "市场里没有可安装的拓展。";
    }
    return `${head}${sections || `<div class="codex-plus-extensions-grid" data-codex-plus-extensions-list="true"><div class="codex-plus-extensions-detail-empty" role="status">${escapeHtml(empty)}</div></div>`}`;
  }

  /** 只刷新侧栏和结果区；搜索输入框保持原 DOM 与光标位置。 */
  function refreshCodexPlusExtensionsView() {
    if (codexPlusActiveEntry() !== "extensions") return;
    if (codexPlusExtensionsSelected && !codexPlusExtensionsSelectionDetail()) codexPlusExtensionsSelected = null;
    const active = document.activeElement;
    const focused = active?.getAttribute?.("data-codex-user-script-key");
    const focusedBrowse = active?.getAttribute?.("data-codex-extensions-browse");
    const focusedNav = !!active?.closest?.("[data-codex-plus-page-nav-body]");
    const body = document.querySelector("[data-codex-plus-page-nav-body]");
    if (body) body.innerHTML = renderCodexPlusExtensionsNav();
    const detail = document.querySelector("[data-codex-plus-extensions-detail]");
    if (detail) {
      detail.innerHTML = renderCodexPlusExtensionsDetail();
      // typeof 守卫：测试会把这段单独切出来执行，那时设置面板的宿主不在作用域里。
      if (typeof mountCodexPlusSettingsPanel === "function") mountCodexPlusSettingsPanel(detail);
    }
    if (focused) {
      Array.from(detail?.querySelectorAll("[data-codex-user-script-key]") || [])
        .find((button) => button.getAttribute("data-codex-user-script-key") === focused)?.focus({ preventScroll: true });
    } else if (focusedBrowse) {
      Array.from((focusedNav ? body : detail)?.querySelectorAll("[data-codex-extensions-browse]") || [])
        .find((button) => button.getAttribute("data-codex-extensions-browse") === focusedBrowse)?.focus({ preventScroll: true });
    }
  }

  /**
   * 解析当前选中项，拿到本地脚本与市场条目两边的信息。
   *
   * 已安装的市场脚本，本地清单里有 `market_id`，据此把市场的描述/作者等补上；
   * 纯本地脚本则只有本地那几个字段。
   */
  function codexPlusExtensionsSelectionDetail() {
    const sel = codexPlusExtensionsSelected;
    if (!sel) return null;
    const local = sel.kind === "installed"
      ? (codexPlusUserScripts.scripts || []).find((script) => script.key === sel.key) || null
      : null;
    const marketItem = sel.kind === "market" ? codexPlusMarketItemById(sel.key) : codexPlusLocalMarketItem(local);
    if ((sel.kind === "installed" && !local) || (sel.kind === "market" && !marketItem)) return null;
    return { sel, local, marketItem };
  }

  function extensionIconSvg() {
    return `<svg viewBox="0 0 16 16" fill="currentColor"><path d="${codexPlusDefaultExtensionIconPath}"/></svg>`;
  }

  /**
   * 条目图标：市场清单的 `icon`，或拓展包清单 codexpp.json 的 `icon`（后台已换成 data URL / https 地址）；
   * 都没有就回退 VSCode 的默认扩展字形。
   *
   * 回退是必须的——清单里的老条目没有这个字段，而且 icon 指的是外链，
   * 加载失败时若不兜底就会留一块空白。失败替换交给委托监听（见 handleExtensionIconError），
   * 不用内联 onerror，避免在字符串拼 HTML 时引入另一处转义面。
   */
  function extensionIconMarkup(icon) {
    const url = String(icon || "").trim();
    if (!url) return extensionIconSvg();
    return `<img class="codex-plus-extensions-icon-img" src="${escapeHtml(url)}" alt="" loading="lazy" />`;
  }

  /**
   * 图标加载失败时换回默认字形。
   *
   * `error` 事件不冒泡，只能在捕获阶段用委托收到；换掉节点本身即可，
   * 再失败也不会递归——替换出来的 svg 不触发 error。
   */
  function handleExtensionIconError(event) {
    const img = event.target;
    if (!(img instanceof HTMLImageElement) || !img.classList.contains("codex-plus-extensions-icon-img")) return;
    const holder = document.createElement("span");
    holder.innerHTML = extensionIconSvg();
    const svg = holder.firstElementChild;
    if (svg) img.replaceWith(svg);
  }

  /** 原生风格详情：居中展示图标、名称、介绍与管理操作。 */
  function renderCodexPlusExtensionsDetail() {
    const detail = codexPlusExtensionsSelectionDetail();
    if (!detail) {
      codexPlusExtensionsSelected = null;
      return renderCodexPlusExtensionsOverview();
    }
    const { sel, local, marketItem } = detail;
    const name = marketItem?.name || local?.name || sel.key;
    const version = marketItem?.version || local?.version || "";
    const author = marketItem?.author || marketItem?.repo?.split("/")[0] || "";
    const description = marketItem?.description || local?.description || local?.package?.description || "";
    const tags = marketItem?.tags || [];
    const requirements = marketItem?.requirements || [];
    const limitations = marketItem?.limitations || [];
    const homepage = marketItem?.homepage || local?.homepage || "";
    const isInstalled = sel.kind === "installed";
    // 拓展包按锁定提交比对（后端算好，本机源码不算有更新）；单文件脚本按版本号比对。
    const updateAvailable = marketItem?.kind === "package"
      ? isInstalled && !!marketItem.updateAvailable
      : isInstalled && marketItem && version && local?.version && local.version !== version;

    // 详情沿用列表的内容宽度，管理操作保持独立。
    const sourceLabel = local ? (typeof codexPlusScriptSourceLabel === "function" ? codexPlusScriptSourceLabel(local) : local.source === "builtin" ? "内置" : "用户") : "";
    const publisherLine = [
      author ? escapeHtml(author) : "",
      version ? `v${escapeHtml(version)}` : "",
      local ? `${escapeHtml(sourceLabel)} · ${escapeHtml(userScriptStatusLabel(local.status))}` : "",
    ].filter(Boolean).join('<span class="codex-plus-extensions-detail-sep">·</span>');

    const actions = [];
    if (isInstalled) {
      actions.push(`
        <button type="button" class="codex-plus-toggle" data-codex-user-script-key="${escapeHtml(local?.key || "")}" data-enabled="${String(!!local?.enabled)}" role="switch" aria-checked="${String(!!local?.enabled)}" aria-label="启用 ${escapeHtml(name)}"><span></span></button>
      `);
      // 内置脚本在只读目录里，删不掉；只给用户目录的脚本提供卸载。
      // 精选拓展包有审核过的新提交：和管理器一样直接给「更新」，走同一条确认安装流程。
      if (updateAvailable && marketItem?.kind === "package") {
        actions.push(`<button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-primary" data-codex-package-install="${escapeHtml(marketItem.id)}"${codexPlusPackageInstalling ? " disabled" : ""}>${codexPlusPackageInstalling === marketItem.id ? "正在准备…" : "更新"}</button>`);
      }
      if (local?.source === "user") {
        actions.push(`<button type="button" class="codex-plus-extensions-detail-button" data-codex-extensions-uninstall="${escapeHtml(local.key)}">卸载</button>`);
      }
      // 和 VS Code 一样，头部的齿轮直达「设置」页签。
      if (typeof codexPlusExtensionHasSettings === "function" && codexPlusExtensionHasSettings(local)) {
        actions.push(`<button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-gear" data-codex-extensions-tab="settings" title="设置" aria-label="${escapeHtml(name)} 的设置">${codexPlusDiscoveryIcons.settings}</button>`);
      }
    } else if (marketItem?.kind === "package") {
      const installing = codexPlusPackageInstalling === marketItem.id;
      actions.push(`<button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-primary" data-codex-package-install="${escapeHtml(marketItem.id)}"${codexPlusPackageInstalling ? " disabled" : ""}>${installing ? "正在准备…" : "安装"}</button>`);
    } else if (marketItem) {
      actions.push(`<button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-primary" data-codex-market-install="${escapeHtml(marketItem.id)}">安装</button>`);
    }

    // 页签（细节 / 设置 / 功能）与右侧栏见 41-extension-detail.js；测试单独切出这段执行时退回到简单正文。
    const list = (title, items) => items.length
      ? `<div class="codex-plus-extensions-detail-section"><div class="codex-plus-extensions-detail-section-title">${escapeHtml(title)}</div><ul>${items.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></div>`
      : "";
    const body = typeof renderCodexPlusExtensionDetailTabs === "function"
      ? renderCodexPlusExtensionDetailTabs({ local, marketItem, sourceLabel })
      : `<div class="codex-plus-extensions-detail-body">
        ${tags.length ? `<div class="codex-plus-extensions-detail-tags">${tags.map((tag) => `<span>${escapeHtml(tag)}</span>`).join("")}</div>` : ""}
        ${list("使用要求", requirements)}
        ${list("已知限制", limitations)}
        ${homepage ? `<div class="codex-plus-extensions-detail-link"><a href="${escapeHtml(homepage)}" target="_blank" rel="noreferrer">${escapeHtml(homepage)}</a></div>` : ""}
        ${local?.error ? `<div class="codex-plus-extensions-detail-error">${escapeHtml(local.error)}</div>` : ""}
      </div>`;

    return `
      <button type="button" class="codex-plus-extensions-back" data-codex-extensions-browse="${codexPlusExtensionsFilter}" aria-label="返回拓展列表">${codexPlusDiscoveryIcons.back}<span>返回拓展列表</span></button>
      <div class="codex-plus-extensions-detail-head">
        <div class="codex-plus-extensions-detail-icon" aria-hidden="true">${extensionIconMarkup(codexPlusExtensionIcon(marketItem, local))}</div>
        <div class="codex-plus-extensions-detail-heading">
          <div class="codex-plus-extensions-detail-title">${escapeHtml(name)}${codexPlusOfficialBadge(marketItem)}</div>
          ${publisherLine ? `<div class="codex-plus-extensions-detail-meta">${publisherLine}</div>` : ""}
          ${description ? `<div class="codex-plus-extensions-detail-description">${escapeHtml(description)}</div>` : ""}
          ${updateAvailable ? `<div class="codex-plus-extensions-detail-update">有新版本 ${escapeHtml(marketItem?.tag || `v${version}`)} 可更新</div>` : ""}
        </div>
        <div class="codex-plus-extensions-detail-actions">${actions.join("")}</div>
      </div>
      ${body}
    `;
  }

  /** 来源标签：内置 / 官方（随 Codex++ 发布的拓展包）/ 市场 / 用户。 */
  function codexPlusScriptSourceLabel(script) {
    if (script?.source === "builtin") return "内置";
    if (script?.source === "bundled") return "官方";
    return script?.market_id ? "市场" : "用户";
  }

  /**
   * 正挂着的拓展设置面板 { key, node, cleanup }。详情区重画（innerHTML）时把同一个节点搬回新的挂载点，
   * 面板里的状态（滚动、正在输入的内容、加载好的预览）不丢；换了拓展或没有挂载点时才清理。
   */
  let codexPlusMountedSettingsPanel = null;

  function unmountCodexPlusSettingsPanel() {
    const mounted = codexPlusMountedSettingsPanel;
    codexPlusMountedSettingsPanel = null;
    if (!mounted) return;
    if (typeof mounted.cleanup === "function") runCodexPlusExtensionCallback(mounted.key, "settingsPanel.cleanup", mounted.cleanup);
    mounted.node.remove();
  }

  function mountCodexPlusSettingsPanel(root) {
    const slot = root?.querySelector?.("[data-codex-plus-settings-panel]");
    const key = slot?.getAttribute("data-codex-plus-settings-panel") || "";
    const mounted = codexPlusMountedSettingsPanel;
    if (mounted && slot && mounted.key === key && mounted.panel === codexPlusSettingsPanelFor(key)) {
      slot.replaceWith(mounted.node);
      return;
    }
    unmountCodexPlusSettingsPanel();
    const panel = slot && typeof codexPlusSettingsPanelFor === "function" ? codexPlusSettingsPanelFor(key) : null;
    if (!panel) return;
    const local = (codexPlusUserScripts.scripts || []).find((script) => script.key === key);
    const config = local?.package?.config || window.__codexPlusUserScripts?.scripts?.[key]?.manifest?.config || {};
    // 拓展自己调的 saveConfig 只覆盖给出的项：它常常只传改动的那一项，整份替换会把别的项打回默认值。
    const saveConfig = (values) => saveCodexPlusExtensionConfig(key, values, true);
    const result = runCodexPlusExtensionCallback(key, "settingsPanel.render", () => panel.render({ container: slot, config: { ...config }, saveConfig }));
    codexPlusMountedSettingsPanel = { key, node: slot, panel, cleanup: result.ok ? result.value : null };
  }

  /** 拓展设置面板保存配置：后台只收清单里声明过的项；拓展用 onConfigUpdate 接住时原地生效，否则整体热重载。 */
  /** merge 为 true：只覆盖给出的项，其余已存的值保留；默认整份替换（通用表单逐项保存时传的是整份）。 */
  async function saveCodexPlusExtensionConfig(key, values, merge = false) {
    const result = await postJson("/user-scripts/set-config", { key, values: values && typeof values === "object" ? values : {}, merge });
    if (!result || result.status !== "ok") throw new Error(result?.message || "拓展配置没有保存");
    const config = result.config && typeof result.config === "object" ? result.config : {};
    const local = (codexPlusUserScripts.scripts || []).find((script) => script.key === key);
    if (local?.package) local.package.config = config;
    const applied = window.__codexPlusUserScripts?.updateConfig?.(key, config);
    if (applied !== "applied") await reloadUserScriptsAfterChange();
    return config;
  }

  /** 注册表里某个拓展的设置面板变了（热重载、启停）：正显示着它就重画详情。 */
  function codexPlusSettingsPanelChanged(key) {
    if (codexPlusMountedSettingsPanel?.key !== key && !(codexPlusExtensionsSelected?.kind === "installed" && codexPlusExtensionsSelected.key === key)) return;
    if (codexPlusMountedSettingsPanel?.key === key) unmountCodexPlusSettingsPanel();
    if (typeof refreshCodexPlusExtensionsView === "function") refreshCodexPlusExtensionsView();
  }

  /** 卸载用户脚本：删文件 + 清记录，然后刷新两侧。 */
  async function uninstallUserScript(key) {
    if (!key) return;
    await postJson("/user-scripts/delete", { key });
    if (codexPlusExtensionsSelected?.kind === "installed" && codexPlusExtensionsSelected.key === key) {
      codexPlusExtensionsSelected = null;
    }
    await reloadUserScriptsAfterChange();
    refreshCodexPlusExtensionsView();
  }

  /** 启停用户脚本：写开关后热重载，页面里的实例才会真的卸下 / 装上。 */
  async function setUserScriptEnabled(key, enabled) {
    if (!key) return;
    await loadUserScripts("/user-scripts/set-script-enabled", { key, enabled });
    await reloadUserScriptsAfterChange();
  }

  /**
   * 启停 / 卸载之后热重载用户脚本（与管理器的启停流程一致）。
   *
   * 只改开关不会动已经跑起来的拓展实例：不重载的话关掉的拓展界面还挂着，
   * 打开的拓展也要等下次启动才装上。重载失败时仍刷新清单，开关状态至少准确。
   */
  async function reloadUserScriptsAfterChange() {
    const result = await postJson("/user-scripts/reload", {});
    if (result?.status === "unknown") showToast(result.message, { type: "warn" });
    else if (!result?.scripts) showToast(`拓展重载失败：${result?.message || "未知错误"}，重启 Codex 后生效`, { type: "error" });
    await loadUserScripts("/user-scripts/list", {}, result?.status === "unknown");
  }

  /** 左面板的导航项。Codex++ 页面切分组，「拓展」页面列脚本。 */
  /** 左面板内容：拓展页是脚本列表，设置页是分组导航（分组从 root 里已渲染的页面读）。 */
  function renderCodexPlusPageNavItems(tab, root = document) {
    if (tab === codexPlusExtensionsTab) return renderCodexPlusExtensionsNav();
    return renderCodexPlusSettingsNavItems(root);
  }

  /**
   * 页面模式下把单栏内容改造成两栏：左面板 + 右内容区。
   *
   * 只搬动已有的 .codex-plus-modal-body，不重建里面那些 data-codex-* 挂载点，
   * 免得 renderUserScripts / 各类 toggle 的 querySelector 找不到目标。
   */
  function installCodexPlusPageLayout(overlay, tab) {
    if (!overlay || overlay.querySelector(".codex-plus-page-layout")) return;
    const content = overlay.querySelector(".codex-plus-modal-content");
    const body = content?.querySelector(".codex-plus-modal-body");
    if (!content || !body) return;
    const layout = document.createElement("div");
    layout.className = "codex-plus-page-layout";
    const main = document.createElement("div");
    main.className = "codex-plus-page-main";
    // 「拓展」的左面板是脚本列表；设置页的左面板是分组导航（和 Codex 自己的设置页一样，右边一次只看一组）。
    // 推荐内容没有左栏：照管理器首页的推荐区，整页一栏（renderCodexPlusAds）。
    if (tab === codexPlusExtensionsTab) {
      const nav = document.createElement("div");
      nav.className = "codex-plus-page-nav";
      nav.innerHTML = `
        <div class="codex-plus-page-nav-header"><div class="codex-plus-page-nav-title">${codexPlusPageTitle(tab)}</div></div>
        <div class="codex-plus-page-nav-body" data-codex-plus-page-nav-body="true">${renderCodexPlusPageNavItems(tab)}</div>
      `;
      layout.appendChild(nav);
    } else if (tab === "home") {
      const nav = document.createElement("nav");
      nav.className = "codex-plus-page-nav";
      nav.setAttribute("aria-label", "Codex++ 设置分组");
      nav.innerHTML = `
        <div class="codex-plus-page-nav-header" data-codex-plus-settings-nav-header="true"></div>
        <div class="codex-plus-page-nav-body" data-codex-plus-page-nav-body="true">${renderCodexPlusPageNavItems(tab, overlay)}</div>
      `;
      // 版本号与后端状态点放在左栏顶上，右边只留当前分组。
      const header = content.querySelector(".codex-plus-modal-header");
      if (header) nav.firstElementChild.appendChild(header);
      body.querySelector(".codex-plus-settings")?.setAttribute("data-codex-plus-settings-paged", "true");
      layout.appendChild(nav);
    }
    content.appendChild(layout);
    layout.appendChild(main);
    main.appendChild(body);
    if (tab === "home") syncCodexPlusSettingsGroups(overlay);
  }

  /** 记住的组（localStorage），不在了就是第一组。 */
  function codexPlusSectionNavActive({ title, groups }) {
    let active = "";
    try { active = window.localStorage.getItem(`codex-plus-section-nav:${title}`) || ""; } catch {}
    return groups.some((group) => group.id === active) ? active : groups[0]?.id || "";
  }

  /** 页面里一组的外壳：远程控制、微信连接的渲染函数用它把各节分进左栏的组里。 */
  function codexPlusSectionGroupHtml(id, html) {
    return `<div class="codex-plus-settings-group" data-codex-plus-section-group="${escapeHtml(id)}">${html}</div>`;
  }

  /** 页面标题：每个 rail 入口一个名字，和图标栏上的标签保持一致。 */
  function codexPlusPageTitle(tab) {
    if (tab === codexPlusExtensionsTab) return "拓展";
    if (tab === codexPlusSponsorTab) return "推荐内容";
    return "Codex++";
  }

  /**
   * 左面板内容随当前分组刷新（切 tab 后调用）。
   * 不传 tab 时按页面当前分组重画：拓展注册 / 撤销后的统一刷新（codexPlusRefreshExtensionHostsNow）不带参数，
   * 落到 home 会把「拓展」页的左面板换成 Codex++ 主页的导航，启停一个拓展就回不到列表。
   */
  function refreshCodexPlusPageNav(tab = codexPlusModalTab(document.querySelector(".codex-plus-modal-content")?.dataset.codexPlusActiveTab)) {
    const body = document.querySelector("[data-codex-plus-page-nav-body]");
    if (!body) return;
    const title = document.querySelector(".codex-plus-page-nav-title");
    if (title) title.textContent = tab === codexPlusExtensionsTab ? "拓展" : "Codex++";
    body.innerHTML = renderCodexPlusPageNavItems(tab);
    if (tab === "home" && typeof syncCodexPlusSettingsGroups === "function") syncCodexPlusSettingsGroups();
  }

  /**
   * 脚本清单变化后同步左面板。
   *
   * 原来这里还要往「用户脚本」区块的开关与目录文本里写值，那个区块已经删掉，
   * 侧栏显示已安装清单，主区列表与详情由 refreshCodexPlusExtensionsView 同步。
   */
  function renderUserScripts() {
    // 左面板也要跟着刷新，否则脚本的启停/状态变化不会反映到列表上。
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusPageNav(codexPlusExtensionsTab);
  }

  async function loadUserScripts(path = "/user-scripts/list", payload = {}, runtimeUnknown = false) {
    const requestPayload = path === "/user-scripts/list"
      ? { ...payload, runtime_status: window.__codexPlusUserScripts?.scripts || {} }
      : payload;
    const result = await postJson(path, requestPayload);
    if (result?.scripts) {
      if (runtimeUnknown) result.scripts.forEach((script) => { script.status = "unknown"; });
      codexPlusUserScripts = result;
      codexPlusUserScriptsLoaded = true;
      renderUserScripts();
      syncCodexPlusBuiltinExtensionToggles();
      // 已安装状态变了，市场的「已安装/有更新」标记也要跟着刷新。
      if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
    }
  }

  /**
   * 拉市场清单。
   *
   * 清单与「已安装」状态都由后端合并好（见 script_market::market_scripts_payload），
   * 前端只需合并本地脚本清单来显示来源与状态。
   */
  /** 后端市场 payload（script_market::market_scripts_payload）→ 页面状态。 */
  function codexPlusScriptMarketFromPayload(result) {
    return {
      scripts: result.scripts,
      packages: Array.isArray(result.packages) ? result.packages : [],
      loaded: true,
      loading: false,
      message: result.message || "",
    };
  }

  async function loadScriptMarket(force = false) {
    if (codexPlusScriptMarket.loading) return;
    if (codexPlusScriptMarket.loaded && !force) return;
    codexPlusScriptMarket = { ...codexPlusScriptMarket, loading: true };
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
    const result = await postJson("/script-market/list", {});
    if (Array.isArray(result?.scripts)) {
      codexPlusScriptMarket = codexPlusScriptMarketFromPayload(result);
    } else {
      codexPlusScriptMarket = {
        ...codexPlusScriptMarket,
        loaded: true,
        loading: false,
        message: result?.message || "拓展加载失败",
      };
    }
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
  }

  async function installScriptFromMarket(id) {
    if (!id) return;
    const result = await postJson("/script-market/install", { id });
    if (Array.isArray(result?.scripts)) {
      codexPlusScriptMarket = codexPlusScriptMarketFromPayload(result);
    }
    // 后端装完会把本地清单一起带回来，省一次往返。
    if (result?.user_scripts?.scripts) {
      codexPlusUserScripts = result.user_scripts;
      codexPlusUserScriptsLoaded = true;
      renderUserScripts();
    } else {
      await loadUserScripts();
    }
    if (codexPlusExtensionsSelected?.kind === "market" && codexPlusExtensionsSelected.key === id) {
      const installed = (codexPlusUserScripts.scripts || []).find((script) => script.market_id === id);
      if (installed) codexPlusExtensionsSelected = { kind: "installed", key: installed.key };
    }
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
  }

  /** 安装确认框里的能力清单，按风险从高到低（与管理器 permissionLines 同一套文案，改一边要同步改另一边）。 */
  function codexPlusPackagePermissionLines(permissions = {}) {
    const lines = [];
    const list = (items) => (Array.isArray(items) ? items : []).join("、");
    if (permissions.backend) {
      lines.push(`常驻后台进程 ${permissions.backend}：可读写本机文件、访问网络`);
      if (permissions.backendHttp) lines.push("后台处理本机 HTTP 请求：本机其它程序可以调用它");
      if (permissions.schedules) lines.push(`定时任务 ${permissions.schedules} 条：按时调用它的后台`);
    }
    if (permissions.launchBefore) lines.push(`启动前脚本 ${permissions.launchBefore}：每次启动 Codex 前在本机运行，可读写本机文件、访问网络`);
    if (permissions.launchAfter) lines.push(`退出后脚本 ${permissions.launchAfter}：每次 Codex 退出后在本机运行，可读写本机文件、访问网络`);
    if (permissions.mcpServers?.length) lines.push(`MCP 服务器 ${list(permissions.mcpServers)}：写进 Codex 配置，由 Codex 在本机启动它指定的命令`);
    if (permissions.dsh) lines.push(`dsh 插件 ${permissions.dsh}：插件代码在本机常驻进程里运行`);
    for (const hook of permissions.hooks || []) {
      lines.push(hook.matcher ? `Codex 钩子 ${hook.event}（匹配 ${hook.matcher}）：可拦截或改写工具调用` : `Codex 钩子 ${hook.event}：可拦截或改写工具调用`);
    }
    if (permissions.toolPipeline) lines.push(`工具调用管线 ${permissions.toolPipeline} 个：可改参数、拦截、代为执行工具调用或改写结果`);
    if (permissions.gatewayRequest) lines.push(`网关请求中间件 ${permissions.gatewayRequest} 个：可读取并改写发往模型的完整请求`);
    if (permissions.gatewayResponse) lines.push(`网关响应中间件 ${permissions.gatewayResponse} 个：可读取并改写模型回复`);
    if (permissions.gatewayError) lines.push(`网关失败重试 ${permissions.gatewayError} 个：请求失败后可改写并重发`);
    if (permissions.gatewayReplaces?.length) {
      lines.push(permissions.gatewayReplaces.includes("builtin:auth-headers")
        ? `关掉内置网关环节 ${list(permissions.gatewayReplaces)}：由它接管，其中鉴权头会把供应商 Key 交给它`
        : `关掉内置网关环节 ${list(permissions.gatewayReplaces)}：由它接管`);
    }
    if (permissions.providers?.length) lines.push(`模型供应商 ${list(permissions.providers)}：并入供应商池，选用时请求发往它指定的地址`);
    if (permissions.tools?.length) lines.push(`模型工具：${permissions.tools.join("、")}`);
    if (permissions.launchArgs?.length) lines.push(`Codex 启动参数：${permissions.launchArgs.join(" ")}`);
    if (permissions.instructions) lines.push("全局指令：追加到 Codex 的系统指令");
    if (permissions.skills?.length) lines.push(`Skills：${list(permissions.skills)}（同步进 Codex 的技能目录）`);
    if (permissions.agents?.length) lines.push(`子 Agent：${list(permissions.agents)}`);
    if (permissions.prompts?.length) lines.push(`提示词：${list(permissions.prompts)}`);
    if (permissions.replaces?.length) lines.push(`顶替内置拓展 ${list(permissions.replaces)}：对应的内置功能停用，由它接管`);
    if (permissions.managerReplaces?.length) lines.push(`顶替管理器页面：${list(permissions.managerReplaces)}`);
    if (permissions.managerData?.length) lines.push(`读取管理器数据：${list(permissions.managerData)}`);
    for (const route of permissions.routes || []) lines.push(`本地路由 ${route}`);
    if (permissions.preload?.length) lines.push(`页面最早期脚本 ${list(permissions.preload)}：在 Codex 自己的脚本之前运行`);
    if (permissions.renderer?.length) lines.push(`在 Codex 页面里运行脚本：${permissions.renderer.join("、")}`);
    return lines;
  }

  /**
   * 安装前的确认框（与管理器 InstallDialog 同一套说明）：审核渠道、实际版本、它要的能力、同名冲突。
   * 返回 `{ value: "install" | "enable" }`，取消时为 null。精选 / 社区安装共用。
   */
  function codexPlusConfirmPreparedInstall(prepared, fallbackName) {
    const trust = prepared.channel === "official"
      ? "Codex++ 官方拓展，安装的是审核过的版本。"
      : prepared.channel === "curated"
      ? "经 Codex++ 审核的精选拓展，安装的是审核过的那一版；作者之后的改动要再审核才会更新。"
      : "⚠ 未经 Codex++ 审核，作者可以在新版本里加入任意代码。只安装你信任的作者的拓展。";
    const sha = String(prepared.sha || "").slice(0, 7);
    const version = prepared.fromDefaultBranch
      ? `${prepared.repo} · 作者未发布版本，将安装默认分支的提交 ${sha}`
      : `${prepared.repo} · 版本 ${prepared.tag || prepared.version}，锁定提交 ${sha}`;
    // 本机已装着同 id 的包时，旧版本没有的能力标成「新增」（按文案逐条比，与管理器一致）。
    const before = prepared.previousPermissions ? new Set(codexPlusPackagePermissionLines(prepared.previousPermissions)) : null;
    const lines = codexPlusPackagePermissionLines(prepared.permissions).map((line) => (before && !before.has(line) ? `【新增】${line}` : line));
    const addedCount = before ? lines.filter((line) => line.startsWith("【新增】")).length : 0;
    const conflict = prepared.conflict
      ? prepared.conflict.source === "github" && prepared.conflict.channel && prepared.conflict.channel !== "community" && prepared.channel === "community"
        ? `本机已装 ${prepared.conflict.repo || ""} 的${prepared.conflict.channel === "official" ? "官方" : "精选"}版本 ${prepared.id}，社区里的同 id 拓展不能替换它。`
        : prepared.conflict.source === "github"
        ? `本机已有来自 ${prepared.conflict.repo || ""} 的同名拓展 ${prepared.id}，安装会替换它（旧版本移到备份目录）。`
        : `本机已有手动或对话生成的同名拓展 ${prepared.id}，安装会替换它（旧版本移到备份目录）。`
      : "";
    const message = [
      trust,
      version,
      "",
      "它会获得这些能力：",
      ...(lines.length ? lines.map((line) => `• ${line}`) : ["没有声明任何能力。"]),
      ...(addedCount ? ["", `相比本机已装的版本，这一版新增了 ${addedCount} 项能力（上面标了【新增】）。`] : []),
      ...(conflict ? ["", conflict] : []),
    ].join("\n");
    const verb = prepared.conflict ? "替换并安装" : "安装";
    return codexPlusAsk({
      title: `安装 ${prepared.name || fallbackName}`,
      message,
      options: [{ value: "install", label: `${verb}（暂不启用）` }, { value: "enable", label: `${verb}并启用` }],
    });
  }

  /**
   * 安装精选 / 官方拓展包：后端按清单锁定的提交下载并暂存 → 确认框列出能力 → 用户确认后放进拓展目录。
   * 与管理器「精选」栏的安装是同一条流程（extension_discovery::CommunityStore）。
   */
  async function installPackageFromMarket(id) {
    if (!id || codexPlusPackageInstalling) return;
    const item = (codexPlusScriptMarket.packages || []).find((entry) => entry.id === id);
    const name = item?.name || id;
    codexPlusPackageInstalling = id;
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
    try {
      const prepared = await postJson("/script-market/package/prepare", { id });
      if (!prepared?.token) {
        showToast(`准备安装 ${name} 失败：${prepared?.message || "未知错误"}`, { type: "error" });
        return;
      }
      const answer = await codexPlusConfirmPreparedInstall(prepared, name);
      if (!answer) {
        void postJson("/script-market/package/discard", { token: prepared.token });
        return;
      }
      const result = await postJson("/script-market/package/install", {
        token: prepared.token,
        enable: answer.value === "enable",
        replace: !!prepared.conflict,
      });
      if (!Array.isArray(result?.scripts)) {
        showToast(`安装 ${name} 失败：${result?.message || "未知错误"}`, { type: "error" });
        return;
      }
      codexPlusScriptMarket = codexPlusScriptMarketFromPayload(result);
      const installed = result.installed || {};
      const tips = [answer.value === "enable" ? `${name} 已安装并启用` : `${name} 已安装，尚未启用`];
      if (installed.hooksError) tips.push(`Codex 钩子配置未能同步：${installed.hooksError}`);
      else if (installed.hooksAdded) tips.push("Codex 配置里新增了「Codex++ 拓展钩子」，请在 Codex 的 Hooks 设置里信任一次，新开对话生效");
      showToast(tips.join("。"), installed.hooksError ? { type: "warn" } : null);
      // 装好后热重载，拓展立刻跑起来（与启停流程一致）；重载会顺带刷新本地清单。
      await reloadUserScriptsAfterChange();
      if (codexPlusExtensionsSelected?.kind === "market" && codexPlusExtensionsSelected.key === id) {
        const local = (codexPlusUserScripts.scripts || []).find((script) => script.key === installed.key);
        if (local) codexPlusExtensionsSelected = { kind: "installed", key: local.key };
      }
    } finally {
      codexPlusPackageInstalling = "";
      if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
    }
  }

  /**
   * 市场条目与本地脚本合并后的视图。
   *
   * 本地有市场装来的脚本（带 market_id），据此判断「已安装」并给出卸载入口；
   * 「市场」分组只列还没装的，避免同一脚本出现两次。
   */
  function codexPlusExtensionsEntries() {
    const local = codexPlusUserScripts.scripts || [];
    const marketById = new Map((codexPlusScriptMarket.scripts || []).map((item) => [item.id, item]));
    const localByMarketId = new Map(
      local.filter((script) => script.market_id).map((script) => [script.market_id, script]),
    );
    const installed = local.map((script) => ({
      kind: "installed",
      key: script.key,
      name: marketById.get(script.market_id)?.name || script.name || script.key,
      meta: `${typeof codexPlusScriptSourceLabel === "function" ? codexPlusScriptSourceLabel(script) : script.source === "builtin" ? "内置" : script.market_id ? "市场" : "用户"} · ${userScriptStatusLabel(script.status)}`,
      enabled: !!script.enabled,
      script,
    }));
    // 和管理器「精选」一样：拓展包（官方在前，后端已排好）排在单文件脚本前面；已装的（含本机源码）不再列。
    const packages = (codexPlusScriptMarket.packages || [])
      .filter((item) => !item.installed && !item.localSource)
      .map((item) => ({
        kind: "market",
        key: item.id,
        name: item.name || item.id,
        meta: `${item.author || item.repo?.split("/")[0] || "未知作者"} · ${item.tag || `v${item.version}`}`,
        item,
      }));
    const sortScripts = (items) => typeof codexPlusSortMarketScripts === "function" ? codexPlusSortMarketScripts(items) : items;
    const scripts = sortScripts((codexPlusScriptMarket.scripts || [])
      .filter((item) => !localByMarketId.has(item.id)))
      .map((item) => ({
        kind: "market",
        key: item.id,
        name: item.name || item.id,
        meta: `${item.author || "未知作者"} · v${item.version}`,
        item,
      }));
    const market = [...packages, ...scripts];
    const available = (codexPlusScriptMarket.scripts || []).filter((item) => localByMarketId.has(item.id));
    // 精选里「已安装」一节：装了的拓展包（含本机源码，有更新的排前面）和市场脚本，点开看本地详情。
    const localByPackageId = new Map(
      local.filter((script) => script.source === "user" && script.package?.id).map((script) => [script.package.id, script]),
    );
    const curatedEntry = (item, script) => script
      ? { kind: "installed", key: script.key, name: item.name || item.id, enabled: !!script.enabled, script, item }
      : { kind: "market", key: item.id, name: item.name || item.id, item };
    const curated = [
      ...(codexPlusScriptMarket.packages || [])
        .filter((item) => item.installed || item.localSource)
        .sort((a, b) => Number(!!b.updateAvailable) - Number(!!a.updateAvailable))
        .map((item) => curatedEntry(item, localByPackageId.get(item.id))),
      ...sortScripts(available).map((item) => curatedEntry(item, localByMarketId.get(item.id))),
    ];
    return { installed, market, available, curated, localByMarketId };
  }

  /**
   * 取条目对应的市场清单项。
   *
   * 市场条目自带 `item`；已安装条目只有本地脚本（`script`），要靠脚本上的
   * `market_id` 回查，否则拿不到图标、简介、标签。纯本地脚本两者都没有。
   */
  function codexPlusExtensionMarketItem(entry) {
    if (entry.item) return entry.item;
    if (entry.kind === "market") return null;
    return codexPlusLocalMarketItem(entry.script);
  }

  /** 本地脚本对应的市场条目：市场脚本靠 `market_id`，精选拓展包靠拓展包 id。 */
  function codexPlusLocalMarketItem(script) {
    const marketId = script?.market_id;
    if (marketId) return (codexPlusScriptMarket.scripts || []).find((item) => item.id === marketId) || null;
    const packageId = script?.package?.id;
    if (!packageId) return null;
    return (codexPlusScriptMarket.packages || []).find((item) => item.id === packageId) || null;
  }

  /** 按 id 找市场条目（单文件脚本或拓展包）。 */
  function codexPlusMarketItemById(id) {
    return (codexPlusScriptMarket.scripts || []).find((item) => item.id === id)
      || (codexPlusScriptMarket.packages || []).find((item) => item.id === id)
      || null;
  }

  // 市场仓库本身：主页指向它的脚本不用作者头像（否则一排全是同一个头像），退回默认图标。
  const codexPlusScriptMarketRepo = "bigpizzav3/codexplusplusscriptmarket";

  /** GitHub 头像地址：`owner/repo` 或 GitHub 仓库地址（与管理器 githubAvatar 一致）。 */
  function codexPlusGithubAvatar(repoOrUrl) {
    const match = /^(?:https?:\/\/github\.com\/)?([A-Za-z0-9-]+)\/([A-Za-z0-9._-]+)/.exec(String(repoOrUrl || "").trim());
    if (!match || `${match[1]}/${match[2]}`.toLowerCase() === codexPlusScriptMarketRepo) return "";
    return `https://github.com/${match[1]}.png?size=64`;
  }

  /**
   * 条目图标，与管理器同一套取法：清单给的图标 → 本地拓展包自带的图标 →
   * 拓展包仓库 / 主页仓库作者的 GitHub 头像；都没有时由 extensionIconMarkup 退回默认字形。
   */
  function codexPlusExtensionIcon(item, local) {
    if (item?.icon) return item.icon;
    if (local?.package?.icon) return local.package.icon;
    if (item?.kind === "package") return codexPlusGithubAvatar(item.repo);
    return /^https?:\/\/github\.com\//i.test(String(item?.homepage || "")) ? codexPlusGithubAvatar(item.homepage) : "";
  }

  /** 「官方」徽标：只有清单标了官方且仓库属于官方账号的拓展包才有（后端已校验）。 */
  function codexPlusOfficialBadge(item) {
    return item?.official ? '<span class="codex-plus-extension-official" title="Codex++ 官方拓展">官方</span>' : "";
  }

  function filterCodexPlusExtensionsEntries(entries) {
    const query = codexPlusExtensionsQuery.trim().toLowerCase();
    if (!query) return entries;
    return entries.filter((entry) => {
      const marketItem = codexPlusExtensionMarketItem(entry);
      return [entry.name, entry.key, entry.script?.name, entry.meta, marketItem?.description, marketItem?.author, marketItem?.repo, ...(marketItem?.tags || [])]
        .filter(Boolean)
        .some((text) => String(text).toLowerCase().includes(query));
    });
  }

  const codexPlusAdsUrl = "/ads";
  let codexPlusAds = [];
  let codexPlusAdsLoaded = false;

  function isCodexPlusAdExpired(ad) {
    if (!ad.expires_at) return false;
    const expiresAt = Date.parse(ad.expires_at);
    return Number.isFinite(expiresAt) && expiresAt < Date.now();
  }

  /**
   * 广告落地页只认 http(s)。
   *
   * 清单来自远程，`javascript:` 之类的 scheme 一旦写进 `<a href>`，点击就会在
   * Codex 渲染进程里执行脚本；`escapeHtml` 只挡得住标签，挡不住 scheme。
   */
  function isCodexPlusAdWebUrl(value) {
    return /^https?:\/\/[^\s/]/i.test(String(value || "").trim());
  }

  /** 清单里的一条推荐：标题、描述、落地页齐全，落地页是 http(s)，没过期。 */
  function normalizeCodexPlusAd(ad) {
    if (!ad || !ad.title || !ad.description || !isCodexPlusAdWebUrl(ad.url) || isCodexPlusAdExpired(ad)) return null;
    return {
      id: String(ad.id || ad.title),
      type: ad.type,
      title: String(ad.title),
      description: String(ad.description),
      url: String(ad.url),
      image: ad.image ? String(ad.image) : "",
      expires_at: ad.expires_at ? String(ad.expires_at) : "",
      highlights: Array.isArray(ad.highlights) ? ad.highlights.map((item) => String(item)).filter(Boolean) : [],
    };
  }

  function normalizeCodexPlusAds(payload) {
    if (!payload || !Array.isArray(payload.ads)) return [];
    // 只留赞助商推荐。上游清单里的 normal 条目不属于赞助商，
    // 放进来只会让「有数据但渲染不出东西」的状态变得难以判断。
    return payload.ads.filter((ad) => ad && ad.type === "sponsor").map(normalizeCodexPlusAd).filter(Boolean);
  }

  /**
   * 置顶赞助位（「精选推荐」的大卡片），和管理器首页同一个来源：本地服务归一化后的 topAds / topAd，
   * 直接拉的原始清单里是 top_ad（单个对象或数组）。
   */
  function normalizeCodexPlusTopAds(payload) {
    const raw = payload?.topAds?.length ? payload.topAds
      : payload?.topAd ? [payload.topAd]
        : Array.isArray(payload?.top_ad) ? payload.top_ad
          : payload?.top_ad ? [payload.top_ad] : [];
    return raw.map(normalizeCodexPlusAd).filter(Boolean);
  }

  let codexPlusTopAds = [];

  function formatCodexPlusAdTitle(title) {
    const value = String(title || "");
    return value.split(/[｜|]/, 1)[0].trim() || value;
  }

  /** 同一家的判据：落地页的域名 + 路径（不看 query、尾斜杠），和管理器首页一致；置顶过的不再列进下面。 */
  function codexPlusAdIdentity(ad) {
    try {
      const url = new URL(ad.url);
      return `${url.host}${url.pathname}`.replace(/\/+$/, "").toLowerCase();
    } catch {
      return String(ad.url || ad.title || "").replace(/[?#].*$/, "").replace(/\/+$/, "").toLowerCase();
    }
  }

  function codexPlusAdHost(url) {
    try {
      return new URL(url).host.replace(/^www\./, "");
    } catch {
      return "";
    }
  }

  /** 卡片右边的外链箭头，跟 VSCode/Codex 的「新窗口打开」同一语义。 */
  const codexPlusAdArrowIcon = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5.5 10.5 10.5 5.5"/><path d="M6.5 5.5h4v4"/></svg>';
  const codexPlusAdCtaIcon = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 8h9"/><path d="M8.5 4l4 4-4 4"/></svg>';

  /** 卡片外壳：整张可点，点了交给系统浏览器（见点击处理里的 data-codex-plus-ad-url）。 */
  function codexPlusAdLinkAttrs(ad, className) {
    return `class="${className}" data-codex-plus-ad-url="${escapeHtml(ad.url)}" href="${escapeHtml(ad.url)}" rel="noreferrer" title="${escapeHtml(`${ad.description}\n${ad.url}`)}"`;
  }

  /** 没有配图时用名称首字当占位，比空一块更整齐。 */
  function codexPlusAdImage(ad, className) {
    const name = formatCodexPlusAdTitle(ad.title);
    return ad.image
      ? `<img class="${className}" src="${escapeHtml(ad.image)}" alt="" draggable="false" />`
      : `<span class="${className} codex-plus-sponsor-placeholder" aria-hidden="true">${escapeHtml(name.slice(0, 1))}</span>`;
  }

  /**
   * 推荐内容页，照管理器首页的推荐区：上面「精选推荐」放置顶位（一条时左图右文横排，多条均分成列），
   * 下面「更多赞助商」两列小卡片（图标、名称 + 两个亮点、一行描述、外链箭头），全部列出、不重复置顶过的。
   */
  function renderCodexPlusAds() {
    if (!codexPlusAdsLoaded) return `<div class="codex-plus-ad-empty">推荐内容加载中…</div>`;
    const heroes = codexPlusTopAds;
    const featured = new Set(heroes.map(codexPlusAdIdentity));
    const rest = codexPlusAds.filter((ad) => !featured.has(codexPlusAdIdentity(ad)));
    if (!heroes.length && !rest.length) return `<div class="codex-plus-ad-empty">暂无推荐内容。</div>`;
    const heroHtml = heroes.map((ad) => {
      const chips = ad.highlights.slice(0, 4).map((item) => `<span>${escapeHtml(item)}</span>`).join("");
      return `<a ${codexPlusAdLinkAttrs(ad, "codex-plus-sponsor-hero")}>
          ${codexPlusAdImage(ad, "codex-plus-sponsor-hero-image")}
          <span class="codex-plus-sponsor-hero-body">
            <strong class="codex-plus-sponsor-hero-title">${escapeHtml(formatCodexPlusAdTitle(ad.title))}</strong>
            <span class="codex-plus-sponsor-hero-text">${escapeHtml(ad.description)}</span>
            ${chips ? `<span class="codex-plus-sponsor-hero-chips">${chips}</span>` : ""}
            <span class="codex-plus-sponsor-hero-foot"><span class="codex-plus-sponsor-hero-host">${escapeHtml(codexPlusAdHost(ad.url))}</span><span class="codex-plus-sponsor-hero-cta">了解更多${codexPlusAdCtaIcon}</span></span>
          </span>
        </a>`;
    }).join("");
    const restHtml = rest.map((ad) => {
      const chips = ad.highlights.slice(0, 2).map((item) => `<span class="codex-plus-sponsor-chip">${escapeHtml(item)}</span>`).join("");
      return `<a ${codexPlusAdLinkAttrs(ad, "codex-plus-sponsor-card")}>
          ${codexPlusAdImage(ad, "codex-plus-sponsor-thumb")}
          <span class="codex-plus-sponsor-body">
            <span class="codex-plus-sponsor-head"><strong>${escapeHtml(formatCodexPlusAdTitle(ad.title))}</strong>${chips}</span>
            <span class="codex-plus-sponsor-desc">${escapeHtml(ad.description)}</span>
          </span>
          <span class="codex-plus-sponsor-go" aria-hidden="true">${codexPlusAdArrowIcon}</span>
        </a>`;
    }).join("");
    return `${heroes.length ? `<section class="codex-plus-sponsor-section">
        <div class="codex-plus-sponsor-section-head"><h2 class="codex-plus-discovery-section-title">精选推荐</h2><span class="codex-plus-sponsor-aside">赞助商推荐：支持 Codex++ 的服务</span></div>
        <div class="codex-plus-sponsor-heroes"${heroes.length === 1 ? ' data-single="true"' : ""}${heroes.length >= 3 ? ' data-dense="true"' : ""} style="grid-template-columns: repeat(${heroes.length}, minmax(0, 1fr))">${heroHtml}</div>
      </section>` : ""}${rest.length ? `<section class="codex-plus-sponsor-section">
        <div class="codex-plus-sponsor-section-head"><h2 class="codex-plus-discovery-section-title">更多赞助商<span class="codex-plus-discovery-section-count">${rest.length}</span></h2></div>
        <div class="codex-plus-sponsor-grid">${restHtml}</div>
      </section>` : ""}`;
  }

  function cacheBustCodexPlusAdUrl(url, version) {
    return `${url}${url.includes("?") ? "&" : "?"}v=${version}`;
  }

  async function directFetchCodexPlusAds() {
    const urls = [
      "https://ads.codexpp.cc/ads.json",
      "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
      "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
    ];
    let lastError = null;
    const cacheBust = Date.now();
    for (const url of urls) {
      try {
        const response = await fetch(cacheBustCodexPlusAdUrl(url, cacheBust), {
          headers: { "Accept": "application/json" },
          cache: "no-store",
        });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return await response.json();
      } catch (error) {
        lastError = error;
      }
    }
    throw lastError || new Error("ad list unavailable");
  }

  async function fetchCodexPlusAds() {
    try {
      const localPayload = await postJson(codexPlusAdsUrl, {});
      let payload = localPayload?.ads ? localPayload : localPayload?.payload;
      codexPlusAds = normalizeCodexPlusAds(payload);
      if (!codexPlusAds.length) {
        payload = await directFetchCodexPlusAds();
        codexPlusAds = normalizeCodexPlusAds(payload);
      }
      codexPlusTopAds = normalizeCodexPlusTopAds(payload);
    } catch (error) {
      sendCodexPlusDiagnostic("ads_fetch_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
      codexPlusAds = [];
      codexPlusTopAds = [];
    } finally {
      codexPlusAdsLoaded = true;
      const panel = document.querySelector('[data-codex-plus-panel="sponsor"] .codex-plus-ad-remote');
      if (panel) panel.innerHTML = renderCodexPlusAds();
    }
  }

  function selectCodexPlusTab(tab) {
    // 归一化后再比对：panel 用的是 extensions，而旧调用点仍传 userScripts，
    // 不统一就会两边都对不上、所有 panel 全被隐藏。
    const normalized = codexPlusModalTab(tab);
    document.querySelectorAll(".codex-plus-modal-content").forEach((modal) => {
      modal.dataset.codexPlusActiveTab = normalized;
    });
    document.querySelectorAll("[data-codex-plus-panel]").forEach((panel) => {
      panel.hidden = codexPlusModalTab(panel.getAttribute("data-codex-plus-panel")) !== normalized;
    });
    if (normalized === codexPlusExtensionsTab) {
      loadUserScripts();
      // 市场清单拉过一次就缓存，切换分组不再重复请求；失败后可从界面手动刷新。
      void loadScriptMarket();
      if (typeof codexPlusExtensionSourceEnsureLoaded === "function") codexPlusExtensionSourceEnsureLoaded(codexPlusExtensionsFilter);
    }
    refreshCodexPlusPageNav(normalized);
  }

  /** 两个 rail 入口各自对应一个页面，激活态要分别判断，不能只看页面开着没有。 */
  function codexPlusActiveEntry() {
    const overlay = document.querySelector(`.${codexPlusPageClass}`);
    if (!overlay) return null;
    const tab = overlay.querySelector(".codex-plus-modal-content")?.dataset?.codexPlusActiveTab;
    if (tab === codexPlusExtensionsTab) return "extensions";
    if (tab === codexPlusSponsorTab) return "sponsor";
    if (tab === codexPlusProviderTab) return "provider";
    if (tab === codexPlusRemoteControlTab) return "remote";
    if (tab === codexPlusWeixinConnectTab) return "weixin";
    if (tab === codexPlusTaskBoardTab) return "tasks";
    return "home";
  }

  function setCodexPlusSidebarNavActive(active, entry = "home") {
    const nav = document.getElementById(codexPlusSidebarNavId);
    const button = nav?.querySelector("button");
    if (button) {
      const on = Boolean(active) && entry === "home";
      button.dataset.active = String(on);
      if (on) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    }
    // Codex++ 入口在设置页和收进它里面的页面（供应商、远程控制、微信连接）打开时都亮。
    [
      [codexPlusRailNavId, ["home", ...codexPlusFoldedPages().map((page) => page.entry)]],
      [codexPlusRailTasksId, ["tasks"]],
      [codexPlusRailExtensionsId, ["extensions"]],
      [codexPlusRailSponsorId, ["sponsor"]],
    ].forEach(([id, entries]) => {
      const railButton = document.querySelector(`#${id} > button`);
      if (!railButton) return;
      const on = Boolean(active) && entries.includes(entry);
      railButton.dataset.active = String(on);
      if (on) railButton.setAttribute("aria-current", "page");
      else railButton.removeAttribute("aria-current");
      // 原生 rail 按钮的选中色由 data-selected 驱动（且需无 data-suppress-active-style）。
      if (on) railButton.setAttribute("data-selected", "");
      else railButton.removeAttribute("data-selected");
    });
    syncCodexPlusRailNativeSelection();
  }

  /**
   * 我们的页面是叠加在 Codex 上的，Codex 不知道，所以它自己那个 destination
   * 的选中态会一直留着，表现为 rail 上同时亮两个。页面打开时给根节点打标记，
   * 由 CSS 把原生选中项压成未选中；关掉即移除标记，原生状态自动恢复。
   *
   * 未选中的颜色取自当前主题下真实的未选中项，避免把深浅色写死。
   */
  function syncCodexPlusRailNativeSelection() {
    const root = document.documentElement;
    if (!root) return;
    const open = Boolean(document.querySelector(`.${codexPlusPageClass}`));
    syncCodexPlusPagePopupStyle(open);
    if (!open) {
      root.removeAttribute("data-codex-plus-page-open");
      root.style.removeProperty("--codex-plus-rail-dim");
      return;
    }
    const rail = document.querySelector(codexPlusRailSelector);
    const unselected = rail?.querySelector(`${codexPlusRailDestinationSelector}:not([aria-current="page"])`);
    const dim = unselected ? getComputedStyle(unselected).color : "";
    if (dim) root.style.setProperty("--codex-plus-rail-dim", dim);
    root.setAttribute("data-codex-plus-page-open", "");
  }

  function positionCodexPlusPage(overlay) {
    if (!overlay?.classList?.contains(codexPlusPageClass)) return;
    const sidebar = document.querySelector("aside.app-shell-left-panel");
    const rect = sidebar?.getBoundingClientRect?.();
    const main = document.querySelector("main");
    const rail = document.querySelector(codexPlusRailSelector);
    const railRect = rail?.getBoundingClientRect?.();
    // 新版：页面要顶替原生侧边栏——从图标栏右边界起铺满，把宽面板整个盖住，
    // 而不是并排在其右侧多出一列（那样会变成「图标栏 + 会话列表 + 我们的面板」三段）。
    // 旧版没有图标栏，我们的入口就在 aside 内部，此时退回 aside 右边界。
    const left = railRect && railRect.width > 0
      ? Math.max(0, railRect.right)
      : (rect && rect.width > 0 ? Math.max(0, rect.right) : 0);
    // 量出来的是视觉坐标，而 overlay 在缩放空间里布局，所以统一折算成布局坐标。
    // CSS 的 calc(100vw / zoom - left) 用的也是这个空间的量，两边才配得上。
    const zoom = codexPlusWindowZoom();
    const layoutLeft = zoom === 1 ? left : left / zoom;
    overlay.style.setProperty("--codex-plus-page-left", `${layoutLeft}px`);
    overlay.style.left = `${layoutLeft}px`;
    const pageSurface = document.querySelector('[class*="_PageSurface_"], [class^="PageSurface-"], [class*=" PageSurface-"]');
    // 26.1117 起的外壳没有 _PageSurface_：原生内容面板 [data-app-shell-surface-backing] 从窗口顶边铺起，
    // 顶部那条标题栏（侧栏开关、前进后退，可拖动窗口）是透明地叠在面板与侧栏上的。
    // overlay 若仍从标题栏下沿开始，上面就露出一条「侧栏那段发灰、面板那段发黑、拐角落在侧栏右边」的
    // 原生底色——用户截图圈出的「顶上多出来一块黑的」就是它。这种外壳下改成和原生面板一样从标题栏
    // 顶边铺起，由 CSS 把标题栏抬到 overlay 之上，内容整体下移标题栏的高度，按钮和拖动都照旧。
    // 外壳里常同时挂着两个标题栏，前一个是 0×0 的空壳，要取真正有尺寸的那个。
    const titlebar = pageSurface
      ? null
      : Array.from(document.querySelectorAll("[data-app-shell-titlebar]"))
        .find((node) => node.getBoundingClientRect().height > 0);
    const titlebarRect = titlebar?.getBoundingClientRect?.();
    const backing = titlebarRect && titlebarRect.height > 0
      ? document.querySelector("[data-app-shell-surface-backing]")
      : null;
    const backingRect = backing?.getBoundingClientRect?.();
    const underTitlebar = Boolean(backingRect && backingRect.height > 0 && backingRect.top <= titlebarRect.top);
    // 旧外壳：官方顶部有一条 header（返回/前进/隐藏侧边栏），图标栏与侧边栏都从它的下沿开始。
    // 我们的 overlay 若从 y=0 铺满就会把整条 header 盖住——用户反馈「比官方少了一条顶部栏」
    // 就是这个原因。这里同样量图标栏的顶边（而非硬编码高度），让 overlay 从 header 下沿开始。
    const top = underTitlebar
      ? Math.max(0, titlebarRect.top)
      : railRect && railRect.height > 0
        ? Math.max(0, railRect.top)
        : (rect && rect.height > 0 ? Math.max(0, rect.top) : 0);
    const layoutTop = zoom === 1 ? top : top / zoom;
    overlay.style.setProperty("--codex-plus-page-top", `${layoutTop}px`);
    overlay.style.top = `${layoutTop}px`;
    const insetTop = underTitlebar ? Math.max(0, titlebarRect.bottom - top) : 0;
    overlay.style.setProperty("--codex-plus-page-inset-top", `${zoom === 1 ? insetTop : insetTop / zoom}px`);
    if (underTitlebar) overlay.dataset.codexPlusTitlebar = "inset";
    else delete overlay.dataset.codexPlusTitlebar;
    // 右侧与下方官方各留了一圈槽：整行的 [data-app-shell-workspace-row] 比视口小
    // （真机 1715x984 / 视口 1719x988，即右、下各 4px），官方内容面板正好收在行的右下角。
    // 我们原先 right/bottom 都贴 0，于是比官方多占这 4px。这里量取而不是硬编码 4。
    const row = document.querySelector("[data-app-shell-workspace-row]");
    const rowRect = row?.getBoundingClientRect?.();
    const rightGutter = rowRect && rowRect.width > 0 ? Math.max(0, window.innerWidth - rowRect.right) : 0;
    const bottomGutter = rowRect && rowRect.height > 0 ? Math.max(0, window.innerHeight - rowRect.bottom) : 0;
    const layoutRight = zoom === 1 ? rightGutter : rightGutter / zoom;
    const layoutBottom = zoom === 1 ? bottomGutter : bottomGutter / zoom;
    overlay.style.setProperty("--codex-plus-page-right", `${layoutRight}px`);
    overlay.style.setProperty("--codex-plus-page-bottom", `${layoutBottom}px`);
    overlay.style.right = `${layoutRight}px`;
    overlay.style.bottom = `${layoutBottom}px`;
    // 圆角同样量取官方面板自身的值，不写死 12px。
    // 这里量的是 _PageSurface_：官方那个与我们 overlay 同格子的页面面板（rect 都是
    // [52, 44, 1663, 940]），它四角同为 12px，左侧那一角也真实可见——真机像素扫描确认
    // 官方左边缘从 y=44 的 x=62 收到 y=54 的 x=52，是一条完整的弧。
    // 别改用 main[data-app-shell-main-surface] 的 --app-shell-main-surface-clip-start-radius：
    // 那个元素左边缘在 362（缩在侧边栏后面），左侧还用 inset 负内缩把圆角裁掉，start 恒为 0，
    // 会让人误判左侧不该圆——第一版就是这么写错的，用户反馈「少一个圆角」正是缺了左边两个角。
    // 类名是 CSS Modules 的哈希名，但 PageSurface 这个名字稳定，且不会命中 PageSurfaceLayout
    //（新旧两种哈希格式 `_PageSurface_xx` / `PageSurface-xx` 都认，#2893）。量到的是视觉值，同样折算成布局坐标。
    // 读不到就保持 0，不做猜测。
    // 新外壳没有 _PageSurface_，量原生面板 backing：它只圆起始一侧（右、下贴着窗口边）。
    // 侧栏开着时 backing 起点在侧栏右边、比我们靠右，我们这块就像面板提前从图标栏边上开始，左侧照它圆；
    // 侧栏收起时 backing 一直铺到图标栏底下，我们与它连成一片，左侧不圆，免得多出一道拐角。
    const pageSurfaceStyle = pageSurface ? getComputedStyle(pageSurface) : null;
    const mainSurfaceStyle = (() => {
      const main = document.querySelector("main[data-app-shell-main-surface]");
      return main ? getComputedStyle(main) : null;
    })();
    const radius = underTitlebar
      ? (backingRect.left > left + 1 ? parseFloat(getComputedStyle(backing).borderTopLeftRadius || "") || 0 : 0)
      : parseFloat(pageSurfaceStyle?.borderTopLeftRadius || "") ||
        parseFloat(mainSurfaceStyle?.getPropertyValue("--app-shell-main-surface-clip-end-radius") || "") ||
        0;
    const layoutRadius = zoom === 1 ? radius : radius / zoom;
    overlay.style.setProperty("--codex-plus-page-radius", `${layoutRadius}px`);
    if (underTitlebar) overlay.style.setProperty("--codex-plus-page-radius-end", "0px");
    else overlay.style.removeProperty("--codex-plus-page-radius-end");
    // 新外壳的原生页面整块就是面板一个底色（侧栏收起时连图标栏底下都是它）。
    // 我们的页面底、左栏、顶栏统一取面板的实际颜色，不再用比它深的正文令牌和比它浅的侧栏令牌。
    // 面板被皮肤画成透明时不取，免得页面透出底下的原生内容。
    const surface = underTitlebar ? getComputedStyle(backing).backgroundColor : "";
    if (surface && !/^transparent$|,\s*0\)$|\/\s*0\)$/.test(surface)) {
      overlay.style.setProperty("--codex-plus-shell-surface", surface);
      overlay.dataset.codexPlusSurface = "native";
    } else {
      overlay.style.removeProperty("--codex-plus-shell-surface");
      delete overlay.dataset.codexPlusSurface;
    }
    // 侧栏展开时原生面板从侧栏右边起：记下侧栏宽度，CSS 让有左栏的页面照原生「左灰右黑带圆角」画。
    if (underTitlebar && backingRect.left > left + 1) {
      overlay.dataset.codexPlusSidebar = "open";
      const sidebarWidth = backingRect.left - left;
      overlay.style.setProperty("--codex-plus-native-sidebar-width", `${zoom === 1 ? sidebarWidth : sidebarWidth / zoom}px`);
    } else if (underTitlebar) {
      // 侧栏收起时原生侧栏是悬浮的：鼠标移到图标栏入口上才浮出一张卡片，宽 = 侧栏宽 − 左边 4px 的缝。
      // 收起后原生侧栏仍按原宽排着（只是被裁掉），量它；左栏照着浮（installCodexPlusNavPeek）。
      overlay.dataset.codexPlusSidebar = "floating";
      overlay.style.removeProperty("--codex-plus-native-sidebar-width");
      const navWidth = document.querySelector("[data-sidebar-navigation-root]")?.getBoundingClientRect().width || 0;
      if (navWidth > 4) overlay.style.setProperty("--codex-plus-native-peek-width", `${(zoom === 1 ? navWidth : navWidth / zoom) - 4}px`);
      else overlay.style.removeProperty("--codex-plus-native-peek-width");
    } else {
      delete overlay.dataset.codexPlusSidebar;
      overlay.style.removeProperty("--codex-plus-native-sidebar-width");
      overlay.style.removeProperty("--codex-plus-native-peek-width");
    }
    // 侧栏开合时 aside 本身的尺寸不一定变，原生面板 backing 的宽度一定变，所以也盯着它。
    const surfaceBacking = document.querySelector("[data-app-shell-surface-backing]");
    if (window.__codexPlusPageLayoutObserver &&
        (window.__codexPlusPageLayoutTargets?.main !== main ||
         window.__codexPlusPageLayoutTargets?.sidebar !== sidebar ||
         window.__codexPlusPageLayoutTargets?.rail !== rail ||
         window.__codexPlusPageLayoutTargets?.backing !== surfaceBacking)) {
      window.__codexPlusPageLayoutObserver.disconnect();
      for (const target of new Set([main, sidebar, rail, surfaceBacking])) {
        if (target) window.__codexPlusPageLayoutObserver.observe(target);
      }
      window.__codexPlusPageLayoutTargets = { main, sidebar, rail, backing: surfaceBacking };
    }
  }

  /**
   * 页面开着时 Codex 可能整块重建外壳：图标栏、原生面板都换成新节点。ResizeObserver 还盯着旧节点，
   * 页面就停在重建那一刻量到的位置——实测重建中途量到的全是 0，页面从窗口左上角铺满，
   * 盖住图标栏和标题栏，顶上的切换压在红绿灯底下。每轮扫描比一下量的还是不是现在这批节点，换了就重量。
   */
  function refreshCodexPlusPagePosition() {
    const overlay = document.querySelector(`.${codexPlusPageClass}`);
    if (!overlay) return;
    const targets = window.__codexPlusPageLayoutTargets || {};
    if (targets.rail === document.querySelector(codexPlusRailSelector)
      && targets.backing === document.querySelector("[data-app-shell-surface-backing]")
      && targets.main === document.querySelector("main")) return;
    positionCodexPlusPage(overlay);
  }

  function observeCodexPlusPageLayout(overlay) {
    window.__codexPlusPageLayoutObserver?.disconnect();
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    const reposition = () => {
      if (overlay.isConnected) positionCodexPlusPage(overlay);
    };
    window.__codexPlusPageLayoutObserver = new ResizeObserver(reposition);
    window.__codexPlusPageLayoutTargets = {};
    window.__codexPlusPageResizeHandler = reposition;
    window.addEventListener("resize", reposition);
    positionCodexPlusPage(overlay);
    syncCodexPlusPageBackButton();
    installCodexPlusNavPeek();
  }

  /**
   * 侧栏收起时照原生悬浮：鼠标停在图标栏的哪个入口上，就浮出那个入口自己的侧栏卡片，换一个入口卡片跟着换，
   * 移开一会儿收起。当前页面的入口（以及窗口左边缘 4px）浮出页面自己的左栏（CSS 见 data-codex-plus-sidebar="floating"，
   * 这里打 data-codex-plus-nav-peek）；别的入口——没打开的、或者正在看原生页面时——浮出独立卡片（showCodexPlusRailPeek）。
   * 侧栏展开时不浮，和原生一样。
   * 只认 pointermove / 离开窗口，监听挂在 document 上；同一代注入只装一次，重复注入时换成新一代的处理函数。
   */
  function installCodexPlusNavPeek() {
    if (window.__codexPlusNavPeekHandler && window.__codexPlusNavPeekGeneration === codexPlusBackendGeneration) return;
    document.removeEventListener("pointermove", window.__codexPlusNavPeekHandler, true);
    document.removeEventListener("pointerout", window.__codexPlusNavPeekHandler, true);
    window.__codexPlusNavPeekGeneration = codexPlusBackendGeneration;
    // shown / pending：null 是收起，"page" 是页面自己的左栏，其余是图标栏入口的 id（独立卡片）。
    let shown = null;
    let pending = null;
    let timer = null;
    const pageCard = () => {
      const overlay = document.querySelector(`.${codexPlusPageClass}[data-codex-plus-sidebar="floating"]`);
      const card = overlay?.querySelector(".codex-plus-modal-content > .codex-plus-page-layout > .codex-plus-page-nav");
      return card ? { overlay, card } : null;
    };
    // 原生悬浮卡片开着（收起动画没走完也算）：卡片上带着 popover 底色。
    const nativePeekOpen = () => Boolean(document.querySelector('[data-sidebar-navigation-root][class*="sidebar-popover-background"]'));
    // 原生在入口之间换卡片只换内容、不重播开合动画：卡片已经浮着（我们的或原生的）时换成我们的另一张就直接换（snap）。
    // 只有从无到有、从有到无才淡入淡出。
    const apply = (next, snap) => {
      timer = null;
      const page = pageCard();
      if (next === "page" && !page) next = null;
      if (next !== null && next !== "page" && !showCodexPlusRailPeek(next, true, snap)) next = null;
      if (next === null || next === "page") hideCodexPlusRailPeek(snap);
      if (page) {
        if (snap) page.card.style.transition = "none";
        if (next === "page") page.overlay.dataset.codexPlusNavPeek = "true";
        else delete page.overlay.dataset.codexPlusNavPeek;
        if (snap) {
          void page.card.offsetWidth;
          page.card.style.removeProperty("transition");
        }
      }
      shown = next;
    };
    window.__codexPlusNavPeekHandler = (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const leavingWindow = event.type === "pointerout" && !event.relatedTarget;
      const page = pageCard();
      const peek = document.getElementById(codexPlusRailPeekId);
      let next = null;
      if (!leavingWindow) {
        const railButton = target?.closest("[data-codex-plus-rail] > button");
        if (railButton) {
          // 收进 Codex++ 的页面（供应商等）最左边也是 Codex++ 的分组栏，所以和设置页一样浮它自己的左栏。
          if (codexPlusSidebarFloating()) next = page && railButton.getAttribute("aria-current") === "page" ? "page" : railButton.parentElement.id;
        } else if (page?.card.contains(target)) next = "page";
        else if (peek?.dataset.open === "true" && peek.contains(target)) next = shown;
        else if (page && event.clientX <= 4) next = "page";
      }
      if (next === shown) {
        clearTimeout(timer);
        timer = null;
        return;
      }
      if (timer && next === pending) return;
      clearTimeout(timer);
      pending = next;
      // 卡片浮着时换入口是立刻的。移到原生入口上时我们的马上收；原生卡片要等一下才出来（从无到有，带动画），我们的照常淡出，不留空档。
      const swapping = next !== null && (shown !== null || nativePeekOpen());
      const delay = next === null
        ? (target?.closest(codexPlusRailDestinationSelector) ? 0 : 250)
        : (swapping || page?.card.contains(target) ? 0 : 120);
      timer = setTimeout(() => apply(next, swapping), delay);
    };
    document.addEventListener("pointermove", window.__codexPlusNavPeekHandler, true);
    document.addEventListener("pointerout", window.__codexPlusNavPeekHandler, true);
  }

  /** 侧栏收起：原生面板一直铺到图标栏右边（展开时它从侧栏右边起）。旧外壳没有这块面板，不算。 */
  function codexPlusSidebarFloating() {
    const rail = document.querySelector(codexPlusRailSelector)?.getBoundingClientRect();
    const backing = document.querySelector("[data-app-shell-surface-backing]")?.getBoundingClientRect();
    return Boolean(rail?.width > 0 && backing?.width > 0 && backing.left <= rail.right + 1);
  }

  const codexPlusRailPeekId = "codex-plus-rail-peek";

  /**
   * 图标栏入口的悬浮卡片内容：标题、左栏条目和点一项要做的事（打开那一页并停在那一组）。
   * 条目和页面里的左栏同一套标记，样式也共用（10-style.js 里 .codex-plus-rail-peek-content）。
   * 任务面板、推荐内容没有左栏，没有卡片。
   */
  function codexPlusRailPeekSpec(railId, load = true) {
    // 条目要等后台读完（设置项清单、用户拓展）：读完时卡片还开着就照新数据重画（重画不再触发读取）。
    const refreshAfter = (loader) => {
      if (!load) return;
      Promise.resolve(loader()).then(() => {
        const peek = document.getElementById(codexPlusRailPeekId);
        if (peek?.dataset.open === "true" && peek.dataset.railId === railId) showCodexPlusRailPeek(railId, false);
      }, () => {});
    };
    if (railId === codexPlusRailNavId) {
      if (!codexPlusSettingsCatalog) refreshAfter(loadCodexPlusSettingsCatalog);
      return {
        label: "Codex++ 设置分组",
        titleHtml: `Codex++ ${escapeHtml(codexPlusVersion)}`,
        attrs: ' data-codex-plus-active-tab="home" data-codex-plus-section-nav="true"',
        items: codexPlusSettingsPeekNavItems(),
        pick(button) {
          const link = button.getAttribute("data-codex-plus-page-link");
          if (link) return openCodexPlusFoldedPage(link);
          const id = button.getAttribute("data-codex-plus-settings-nav");
          if (!id) return;
          openCodexPlusPage();
          selectCodexPlusSettingsGroup(id);
        },
      };
    }
    if (railId === codexPlusRailExtensionsId) {
      if (!codexPlusUserScriptsLoaded) refreshAfter(loadUserScripts);
      return {
        label: "拓展",
        titleHtml: escapeHtml(codexPlusPageTitle(codexPlusExtensionsTab)),
        attrs: ` data-codex-plus-active-tab="${codexPlusExtensionsTab}"`,
        items: renderCodexPlusExtensionsNav(),
        pick(button) {
          const select = button.getAttribute("data-codex-extensions-select");
          if (!select && !button.hasAttribute("data-codex-extensions-browse")) return;
          if (!select) codexPlusExtensionsFilter = "market";
          openCodexPlusExtensions();
          if (!select) return;
          const [kind, ...rest] = select.split(":");
          codexPlusExtensionsSelected = { kind, key: rest.join(":") };
          if (typeof resetCodexPlusExtensionsDetailTab === "function") resetCodexPlusExtensionsDetailTab();
          refreshCodexPlusExtensionsView();
        },
      };
    }
    return null;
  }

  /**
   * 设置页没打开时的分组导航：分组外壳照设置页的顺序拼一份（不画里面的设置项），
   * 交给 renderCodexPlusSettingsNavItems，小标题、图标、选中态和页面里的左栏一样。
   */
  function codexPlusSettingsPeekNavItems() {
    const catalog = codexPlusSettingsCatalog;
    const groups = catalog
      ? catalog.groups.map((group) => renderCodexPlusSettingsGroup(group.id, group.title, "")).join("")
      : renderCodexPlusSettingsGroup("core", "常规", "");
    const menu = codexPlusExtensionItems(codexPlusRegistry.menuItems).length ? '<div data-codex-plus-ext-menu="true"></div>' : "";
    const root = document.createElement("div");
    root.innerHTML = `<div data-codex-plus-panel="home">${groups}${renderCodexPlusSettingsSectionGroups()}${renderCodexPlusSettingsGroup("ext-menu", "拓展设置", menu)}${renderCodexPlusSettingsGroup("tools", "工具与关于", "")}</div>`;
    const html = renderCodexPlusSettingsNavItems(root);
    // 开着的是收进 Codex++ 的页面（供应商等）时，选中的是「页面」里那一项，设置分组一个都不选。
    const entry = codexPlusActiveEntry();
    return codexPlusFoldedPages().some((page) => page.entry === entry)
      ? html.replace(/(data-codex-plus-settings-nav="[^"]*") data-active="true" aria-current="page"/g, '$1 data-active="false"')
      : html;
  }

  /**
   * 收进 Codex++ 的页面（供应商、远程控制、微信连接）打开时，最左边照样是 Codex++ 的分组栏，
   * 右边才是那一页自己的布局（供应商：列表 | 详情；远程控制、微信连接：分组 | 内容），
   * 看起来就是在 Codex++ 页面里面换了一页，而不是跳到另一个页面。
   * 页面挂进 body 之后调用：分组栏的选中项按当前开着的页面算。
   * 左栏和设置页同一套样式（modal-content 打 data-codex-plus-section-nav）；侧栏收起时它照样浮成卡片
   * （installCodexPlusNavPeek 认 .codex-plus-modal-content > .codex-plus-page-layout > .codex-plus-page-nav）。
   */
  function installCodexPlusEmbeddedNav(overlay, sectionNav = null) {
    const content = overlay?.querySelector(".codex-plus-modal-content");
    if (!content || overlay.dataset.codexPlusEmbedded === "true") return;
    const pageBody = content.querySelector(".codex-plus-modal-body");
    const layout = document.createElement("div");
    layout.className = "codex-plus-page-layout";
    const main = document.createElement("div");
    main.className = "codex-plus-page-main";
    main.append(...content.childNodes);
    const nav = document.createElement("nav");
    nav.className = "codex-plus-page-nav";
    nav.setAttribute("aria-label", "Codex++ 设置分组");
    nav.innerHTML = `<div class="codex-plus-page-nav-header"><div class="codex-plus-page-nav-title codex-plus-embedded-nav-title"><span class="codex-plus-backend-indicator" data-codex-backend-indicator="true" data-status="${escapeHtml(codexPlusBackendStatus.status || "checking")}"></span><span>Codex++ ${escapeHtml(codexPlusVersion)}</span></div></div>
      <div class="codex-plus-page-nav-body" data-codex-plus-embedded-nav-body="true"></div>`;
    layout.append(nav, main);
    content.appendChild(layout);
    content.dataset.codexPlusSectionNav = "true";
    overlay.dataset.codexPlusEmbedded = "true";
    const render = () => {
      const body = nav.isConnected ? nav.querySelector("[data-codex-plus-embedded-nav-body]") : null;
      if (body) body.innerHTML = codexPlusSettingsPeekNavItems();
    };
    // 这一页自己的分组：右边只显示选中的一组（页面整块重画后，MutationObserver 在绘制前补回 data-active），
    // 选中的组记在同一个 localStorage 键里。
    const section = sectionNav ? { entry: codexPlusActiveEntry(), nav: sectionNav, active: codexPlusSectionNavActive(sectionNav) } : null;
    codexPlusEmbeddedSection = section;
    const syncSection = () => {
      if (!section || !pageBody) return;
      pageBody.querySelectorAll("[data-codex-plus-section-group]").forEach((node) => {
        const value = String(node.getAttribute("data-codex-plus-section-group") === section.active);
        if (node.dataset.active !== value) node.dataset.active = value;
      });
    };
    if (section && pageBody) new MutationObserver(syncSection).observe(pageBody, { childList: true, subtree: true });
    syncSection();
    nav.addEventListener("click", (event) => {
      const button = event.target instanceof Element ? event.target.closest("button") : null;
      if (!button) return;
      // 不让页面自己的点击处理（挂在 overlay 上）再处理一遍。
      event.stopPropagation();
      const sectionId = button.getAttribute("data-codex-plus-embedded-section");
      if (section && sectionId) {
        section.active = sectionId;
        try { window.localStorage.setItem(`codex-plus-section-nav:${section.nav.title}`, sectionId); } catch {}
        syncSection();
        render();
        // 换一组从顶上看起。
        if (pageBody) pageBody.scrollTop = 0;
        return;
      }
      const link = button.getAttribute("data-codex-plus-page-link");
      if (link) return openCodexPlusFoldedPage(link);
      const group = button.getAttribute("data-codex-plus-settings-nav");
      if (!group) return;
      openCodexPlusPage();
      selectCodexPlusSettingsGroup(group);
    });
    render();
    // 设置分组清单要等后台读完；读完时页面还开着就照新清单重画。
    if (!codexPlusSettingsCatalog) Promise.resolve(loadCodexPlusSettingsCatalog()).then(render, () => {});
  }

  /**
   * 浮出入口的独立卡片，位置照原生悬浮侧栏：图标栏右边起，上下左各留 4px，宽 = 原生侧栏宽 − 4。
   * snap：从另一张浮着的卡片换过来，直接出现，不淡入。
   */
  function showCodexPlusRailPeek(railId, load = true, snap = false) {
    const spec = codexPlusRailPeekSpec(railId, load);
    const rail = document.querySelector(codexPlusRailSelector)?.getBoundingClientRect();
    if (!spec || !rail || rail.width <= 0) return false;
    let peek = document.getElementById(codexPlusRailPeekId);
    // 上一代注入留下的卡片绑着旧闭包，换掉。
    if (peek && peek.__codexPlusGeneration !== codexPlusBackendGeneration) {
      peek.remove();
      peek = null;
    }
    if (!peek) {
      peek = document.createElement("div");
      peek.id = codexPlusRailPeekId;
      peek.__codexPlusGeneration = codexPlusBackendGeneration;
      peek.className = "codex-plus-rail-peek";
      markCodexPlusExtensionNode(peek, "builtin-rail-peek");
      peek.addEventListener("click", (event) => {
        const button = event.target instanceof Element ? event.target.closest("button") : null;
        const pick = peek.__codexPlusPick;
        if (!button || !pick) return;
        hideCodexPlusRailPeek(true);
        pick(button);
        // 新开的页面左栏接着浮在鼠标底下，和在原生卡片里点一项一样。
        const overlay = document.querySelector(`.${codexPlusPageClass}[data-codex-plus-sidebar="floating"]`);
        if (overlay) overlay.dataset.codexPlusNavPeek = "true";
      });
      document.body.appendChild(peek);
    }
    applyCodexPlusTheme(peek);
    const zoom = applyCodexPlusZoom(peek);
    const navWidth = document.querySelector("[data-sidebar-navigation-root]")?.getBoundingClientRect().width || 288;
    peek.style.left = `${(rail.right + 4) / zoom}px`;
    peek.style.top = `${(rail.top + 4) / zoom}px`;
    peek.style.bottom = `${(window.innerHeight - rail.bottom + 4) / zoom}px`;
    peek.style.width = `${(navWidth - 4) / zoom}px`;
    peek.innerHTML = `<div class="codex-plus-rail-peek-content"${spec.attrs}><nav class="codex-plus-page-nav" aria-label="${escapeHtml(spec.label)}">
      <div class="codex-plus-page-nav-header"><div class="codex-plus-page-nav-title">${spec.titleHtml}</div></div>
      <div class="codex-plus-page-nav-body">${spec.items}</div></nav></div>`;
    peek.__codexPlusPick = spec.pick;
    peek.dataset.railId = railId;
    if (peek.dataset.open !== "true") {
      // 先按收起的样子排一次，打开才有淡入。
      void peek.offsetWidth;
      if (snap) peek.style.transition = "none";
      peek.dataset.open = "true";
      if (snap) {
        void peek.offsetWidth;
        peek.style.removeProperty("transition");
      }
    }
    return true;
  }

  function hideCodexPlusRailPeek(instant = false) {
    const peek = document.getElementById(codexPlusRailPeekId);
    if (!peek) return;
    if (instant) peek.remove();
    else peek.dataset.open = "false";
  }

  function codexPlusHostUsesLightTheme() {
    const root = document.documentElement;
    const body = document.body;
    const explicitTheme = [
      root?.getAttribute("data-theme"),
      body?.getAttribute("data-theme"),
      root?.getAttribute("data-color-scheme"),
      body?.getAttribute("data-color-scheme"),
    ].filter(Boolean).join(" ").toLowerCase();
    if (/\b(light|light-mode|theme-light)\b/.test(explicitTheme)) return true;
    if (/\b(dark|dark-mode|theme-dark)\b/.test(explicitTheme)) return false;

    const themeClasses = [root?.className, body?.className]
      .filter((value) => typeof value === "string")
      .join(" ")
      .toLowerCase();
    if (/(^|\s)(light|light-mode|theme-light)(\s|$)/.test(themeClasses)) return true;
    if (/(^|\s)(dark|dark-mode|theme-dark)(\s|$)/.test(themeClasses)) return false;

    return !window.matchMedia?.("(prefers-color-scheme: dark)")?.matches;
  }

  /**
   * 取 Codex 当前的界面缩放系数。
   *
   * Codex 的界面缩放的实现是给内层布局节点设 CSS `zoom`（例如 1.2），而不是改
   * documentElement，所以固定在 body 下的 overlay 不会自动跟随。这里把它读出来，
   * 由 applyCodexPlusZoom 自己套上。
   */
  function codexPlusWindowZoom() {
    const raw = getComputedStyle(document.documentElement).getPropertyValue(codexPlusWindowZoomVar);
    const value = Number.parseFloat(raw);
    return Number.isFinite(value) && value > 0 ? value : 1;
  }

  /**
   * 让 overlay 跟随 Codex 的界面缩放，并保持满屏。
   *
   * 直接给 fixed 元素设 zoom 的话，它自身的 `inset: 0` / `100vw` 都还是按未缩放的
   * 视口算，再乘 zoom 就溢出（实测 zoom=1.2 时 100vw 得到 2072px，视口只有 1727）。
   * 所以 overlay 自身的尺寸改用 `calc(100vw / var(--codex-plus-zoom))` 抵消，
   * 内部子元素则用百分比——它们在缩放空间里，百分比本来就对。
   *
   * 字号不在这里动：由 CSS 跟随 zoom 自然放大，与原生行为一致。
   */
  function applyCodexPlusZoom(overlay) {
    if (!overlay?.style) return 1;
    const zoom = codexPlusWindowZoom();
    overlay.style.setProperty("--codex-plus-zoom", String(zoom));
    if (zoom === 1) {
      overlay.style.removeProperty("zoom");
      return 1;
    }
    overlay.style.setProperty("zoom", String(zoom));
    return zoom;
  }

  /**
   * 用 Codex 自己的语义令牌刷新 overlay 的变量。
   *
   * 早先这里内联写死了一套 zinc 调色板（深色正文 #f3f4f6、次要 #a1a1aa），
   * 内联优先级最高，把样式表里本来正确的令牌链整个盖掉了，表现为我们的文字
   * 比原生偏白偏冷。现在改成读宿主算好的值——取不到才回落到兜底色，
   * 这样浅色/深色主题都跟原生同源，也不会在 Codex 调色板变动后失配。
   */
  function applyCodexPlusTheme(overlay) {
    if (!overlay?.style) return;
    const light = codexPlusHostUsesLightTheme();
    const host = getComputedStyle(document.documentElement);
    const read = (names, fallback) => {
      for (const name of names) {
        const value = host.getPropertyValue(name).trim();
        if (value) return value;
      }
      return fallback;
    };
    const variables = {
      "--codex-plus-bg-primary": read(
        ["--color-token-bg-primary", "--token-bg-primary", "--app-color-background-surface"],
        light ? "#ffffff" : "#141414",
      ),
      "--codex-plus-bg-secondary": read(
        ["--color-token-bg-secondary", "--token-bg-secondary", "--color-surface-secondary"],
        light ? "#f7f7f7" : "#2f2f2f",
      ),
      "--codex-plus-bg-elevated": read(
        ["--color-token-dropdown-background", "--color-surface-elevated-secondary", "--color-token-bg-elevated-secondary"],
        light ? "#ffffff" : "#2f2f2f",
      ),
      // 设置页使用独立 surface，正文背景令牌在部分版本中比原生设置页更深。
      "--codex-plus-settings-page-bg": read(
        ["--color-background-surface", "--app-color-background-surface"],
        light ? "#ffffff" : "#181818",
      ),
      "--codex-plus-nav-bg": read(
        ["--color-token-sidebar-background", "--color-background-sidebar"],
        light ? "#f7f7f7" : "#1e1e1e",
      ),
      "--codex-plus-nav-selected-bg": read(
        ["--color-token-interactive-bg-secondary-selected", "--color-background-primary-soft-active"],
        light ? "#e9e9e9" : "#2f2f2f",
      ),
      "--codex-plus-settings-card-bg": read(
        ["--color-background-surface-secondary", "--color-background-surface-raised"],
        light ? "#f7f7f7" : "#232323",
      ),
      // 开关先取原生 Switch 自己的 --switch-* 变量（和 10-style.js 里的定义同一条链）。
      // 不能用 --color-background-accent-solid：新版 Codex 深色主题下它是 #ffffff，开着的开关会变成白底白点。
      "--codex-plus-switch-on": read(
        ["--switch-track-color-checked"],
        light ? "#0169cc" : "#0285ff",
      ),
      "--codex-plus-switch-off": read(
        ["--switch-track-color", "--color-background-switch-off", "--color-background-secondary-solid"],
        light ? "#dedede" : "#3a3a3a",
      ),
      "--codex-plus-switch-thumb": read(
        ["--switch-thumb-color", "--color-background-switch-thumb"],
        "#ffffff",
      ),
      "--codex-plus-bg-hover": read(
        ["--color-token-interactive-bg-secondary-hover", "--color-background-primary-soft-hover", "--token-list-hover-background"],
        light ? "rgba(0,0,0,.06)" : "rgba(255,255,255,.08)",
      ),
      "--codex-plus-bg-selected": read(
        ["--color-token-interactive-bg-secondary-selected", "--color-background-primary-soft-active"],
        light ? "rgba(0,0,0,.08)" : "rgba(255,255,255,.12)",
      ),
      "--codex-plus-text": read(
        ["--color-token-text-primary", "--color-text-primary", "--token-text-primary"],
        light ? "#171717" : "#dfdfdf",
      ),
      "--codex-plus-text-secondary": read(
        ["--color-token-text-secondary", "--color-text-secondary-solid", "--color-text-secondary"],
        light ? "#5d5d5d" : "rgba(255,255,255,.71)",
      ),
      "--codex-plus-text-tertiary": read(
        ["--color-token-text-tertiary", "--color-text-tertiary"],
        light ? "#8a8a8a" : "rgba(255,255,255,.498)",
      ),
      "--codex-plus-border": read(
        ["--color-token-border-default", "--color-token-border", "--color-border-primary-outline"],
        light ? "rgba(0,0,0,.12)" : "rgba(255,255,255,.084)",
      ),
      "--codex-plus-border-subtle": read(
        ["--color-token-border-subtle", "--color-border-disabled"],
        light ? "rgba(0,0,0,.08)" : "rgba(255,255,255,.06)",
      ),
      "--codex-plus-danger": read(
        ["--color-text-danger", "--color-token-text-error"],
        light ? "#dc2626" : "#ff6764",
      ),
      "--codex-plus-success": read(
        ["--color-text-success", "--app-color-text-success"],
        light ? "#15803d" : "#40c977",
      ),
      "--codex-plus-warning": read(
        ["--color-text-warning", "--color-text-caution-surface"],
        light ? "#a16207" : "#ffc300",
      ),
    };
    Object.entries(variables).forEach(([name, value]) => overlay.style.setProperty(name, value));
    overlay.dataset.codexPlusTheme = light ? "light" : "dark";
  }

  /**
   * 规范化页面/tab 名。
   *
   * 用户脚本从 Codex++ 弹窗里拆出来成了独立的「拓展」页面，
   * 这里把旧名 userScripts 也映射过去，避免存量调用点失效。
   */
  function codexPlusModalTab(tab) {
    if (tab === "extensions" || tab === "userScripts") return codexPlusExtensionsTab;
    if (tab === "sponsor") return "sponsor";
    return "home";
  }

  // ── 设置项清单 ──────────────────────────────────────────────────────────────
  // 设置页里每个开关的标题、说明、分组都来自后端 /settings/catalog，
  // 与管理器「Codex → 内置拓展」页 import 的是同一份 JSON（crates/codex-plus-core/assets/enhancement-settings.json）。
  // 每个功能是一个内置拓展：开关就是它对应的设置项，子选项（清单里 parent 指向它）缩进排在它下面；
  // 没有设置项的内置拓展（清单顶层 builtins）经 /user-scripts/set-script-enabled 启停。
  // 这里不写任何一份设置文案：拿不到清单时只显示原因与重试，绝不退回写死的副本。
  let codexPlusSettingsCatalog = null;
  // idle / loading / failed / ok。idle 与 loading 渲染相同的“正在读取”。
  let codexPlusSettingsCatalogStatus = { status: "idle", message: "" };
  let codexPlusSettingsCatalogRequest = null;

  // ── 设置页分组导航 ──────────────────────────────────────────────────────────
  // 页面模式下左栏列出各分组，右边一次只显示一组（data-active），选中的分组记在 localStorage 里。
  // 分组列表从已渲染的 DOM 里读：清单分组、「工具与关于」、有拓展菜单项时的「拓展设置」。
  const codexPlusSettingsGroupStorageKey = "codex-plus-settings-group";
  let codexPlusSettingsActiveGroup = (() => {
    try { return String(window.localStorage.getItem(codexPlusSettingsGroupStorageKey) || ""); } catch { return ""; }
  })();

  function codexPlusSettingsGroupNodes(root = document) {
    const panel = root.querySelector('[data-codex-plus-panel="home"]');
    if (!panel) return [];
    return [...panel.children].filter((node) => node.matches("[data-codex-plus-settings-group]")
      && (node.getAttribute("data-codex-plus-settings-group") !== "ext-menu" || !!node.querySelector("[data-codex-plus-ext-menu]")));
  }

  /** 记住的分组不在了（清单变了、拓展菜单撤了）就落到第一组。 */
  function codexPlusResolvedSettingsGroup(groups) {
    const ids = groups.map((node) => node.getAttribute("data-codex-plus-settings-group"));
    return ids.includes(codexPlusSettingsActiveGroup) ? codexPlusSettingsActiveGroup : ids[0] || "";
  }

  /**
   * 左栏分组图标：直接用 Codex 原生设置页左栏的那套 16px 图标（从原生设置页 DOM 拷出的 SVG，
   * 填充色跟 currentColor），和原生观感一致。值是 [viewBox, 内容]。
   * 清单里新加的分组没登记就用「常规」的齿轮兜底。
   */
  const codexPlusSettingsGroupIcons = {
    "core": ['0 0 16 16', '<path fill-rule="evenodd" clip-rule="evenodd" d="M8.00062 5.2793C9.50689 5.2793 10.7289 6.50063 10.7291 8.00684C10.7289 9.51311 9.50693 10.7354 8.00062 10.7354C6.49463 10.735 5.27328 9.51288 5.27308 8.00684C5.27335 6.50086 6.49467 5.27967 8.00062 5.2793ZM8.00062 6.33008C7.07457 6.33045 6.32413 7.08076 6.32386 8.00684C6.32406 8.93298 7.07453 9.68421 8.00062 9.68458C8.92703 9.68458 9.67816 8.93321 9.67836 8.00684C9.67809 7.08053 8.92699 6.33008 8.00062 6.33008Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M8.59633 1.24024C9.07592 1.24041 9.51987 1.49643 9.75941 1.91309C9.97218 2.28352 10.2906 2.83779 10.5465 3.28223C10.5982 3.37176 10.694 3.42675 10.7985 3.42676L12.3795 3.42774C12.8587 3.42793 13.3021 3.68368 13.5416 4.09864L14.1295 5.11719C14.3672 5.52926 14.3684 6.03653 14.1334 6.4502L13.2536 8.00098L14.1334 9.54786C14.3685 9.96173 14.3675 10.4706 14.1295 10.8828L13.5416 11.9014C13.302 12.3162 12.8586 12.5721 12.3795 12.5723L10.7975 12.5713C10.6934 12.5715 10.5981 12.6276 10.5465 12.7168C10.2906 13.1614 9.97118 13.7156 9.75844 14.0859C9.51881 14.5027 9.07511 14.7588 8.59535 14.7588C8.17471 14.7588 7.8383 14.7578 7.40492 14.7578C6.92515 14.7578 6.48146 14.5027 6.24183 14.0859C6.02905 13.7155 5.70972 13.1614 5.45375 12.7168C5.40213 12.6273 5.3062 12.5713 5.20179 12.5713H3.62172C3.14243 12.5713 2.69927 12.3155 2.45961 11.9004L1.87172 10.8818C1.63401 10.4698 1.63199 9.96251 1.86683 9.54883L2.74769 7.99903L1.86683 6.4502C1.63181 6.03636 1.63375 5.52838 1.87172 5.11622L2.45961 4.09864C2.69922 3.68389 3.14176 3.42795 3.62074 3.42774H5.20179C5.30622 3.42774 5.4021 3.37172 5.45375 3.28223C5.70967 2.83774 6.0281 2.28351 6.24086 1.91309C6.48037 1.49635 6.92432 1.24044 7.40394 1.24024C7.83772 1.24023 8.1753 1.24023 8.59633 1.24024ZM7.40492 2.29004C7.2999 2.29005 7.20281 2.34636 7.15101 2.43653C6.93819 2.80705 6.62008 3.3617 6.3639 3.80665C6.12441 4.22231 5.6814 4.47732 5.20277 4.47754H3.62172C3.51764 4.47754 3.42093 4.533 3.36879 4.62305L2.7809 5.64161C2.72924 5.73111 2.72899 5.84174 2.77992 5.93165L3.80629 7.73926C3.84008 7.79878 3.86295 7.86417 3.87172 7.93165L3.87562 7.99903C3.87565 8.06763 3.86231 8.13613 3.83656 8.19922L3.80629 8.25977L2.77894 10.0674C2.72805 10.1573 2.72924 10.2679 2.7809 10.3574L3.36879 11.376C3.42092 11.4661 3.51761 11.5215 3.62172 11.5215H5.20179C5.68071 11.5215 6.12432 11.7765 6.3639 12.1924C6.62013 12.6374 6.93914 13.1919 7.15199 13.5625C7.20372 13.6524 7.29923 13.7088 7.40394 13.709C7.83771 13.709 8.17531 13.708 8.59633 13.708C8.70115 13.7078 8.79754 13.6525 8.84926 13.5625C9.06202 13.1921 9.38023 12.6382 9.63636 12.1934C9.87592 11.7773 10.3195 11.5215 10.7985 11.5215H12.3795C12.4834 11.5213 12.5794 11.4658 12.6315 11.376L13.2194 10.3574C13.271 10.2679 13.2714 10.1573 13.2203 10.0674L12.193 8.25977C12.1481 8.18049 12.1256 8.09011 12.1256 7.99903C12.1257 7.9079 12.149 7.81751 12.194 7.73829L13.2203 5.93165C13.2712 5.84187 13.2708 5.73203 13.2194 5.64258L12.6315 4.62403C12.5794 4.53379 12.4828 4.47754 12.3786 4.47754L10.7985 4.47852C10.3197 4.47852 9.87697 4.22238 9.63734 3.80665C9.38106 3.36152 9.0621 2.80612 8.84926 2.43555C8.79741 2.3456 8.70021 2.29005 8.59535 2.29004C8.17471 2.29004 7.8383 2.29004 7.40492 2.29004Z" fill="currentColor"></path>'], // 原生「常规」
    "plugins": ['0 0 16 16', '<path fill-rule="evenodd" clip-rule="evenodd" d="M8.25031 1.46094C12.2175 1.46116 14.7053 4.56317 14.4573 8.11328C14.3646 9.44154 13.6395 10.4315 12.6556 10.834C11.7842 11.1903 10.7744 11.0568 9.95637 10.3848C9.43406 10.8255 8.8274 11.1141 8.19465 11.167C7.46206 11.2281 6.74478 10.9691 6.16535 10.3672L6.16145 10.3623C6.16145 10.3623 6.15859 10.3586 6.15656 10.3564C6.15204 10.3517 6.14556 10.344 6.13703 10.335C6.11976 10.3167 6.09427 10.29 6.06281 10.2568C5.9996 10.1901 5.90986 10.0966 5.80793 9.98926C5.60368 9.77412 5.34664 9.50307 5.1341 9.28125C4.86457 8.99958 4.87183 8.55158 5.15363 8.28027L5.31672 8.12207L4.72004 7.50195C4.5193 7.29309 4.52604 6.96077 4.73469 6.75977C4.94359 6.55869 5.27678 6.56454 5.47785 6.77344L6.07453 7.39355L7.51789 6.00391L6.92121 5.38379C6.72021 5.17497 6.72621 4.8427 6.93488 4.6416C7.14378 4.44052 7.47697 4.44638 7.67805 4.65527L8.27473 5.27539L8.44465 5.1123C8.72754 4.84001 9.17872 4.85048 9.44953 5.13477L10.4808 6.21777C11.074 6.8285 11.3084 7.55474 11.2132 8.28613C11.1518 8.75707 10.9544 9.20384 10.6683 9.60938C11.1897 10.0141 11.7752 10.0597 12.2581 9.8623C12.8307 9.6281 13.3423 9.01591 13.4105 8.04004C13.6191 5.05211 11.5645 2.51194 8.25031 2.51172C5.3194 2.51172 2.78634 4.7507 2.58918 7.57031C2.36888 10.7251 4.6005 13.3876 7.99836 13.3877C9.02878 13.3877 10.0514 13.1687 10.8314 12.7041C11.0805 12.5558 11.4027 12.6377 11.5511 12.8867C11.6992 13.1357 11.6174 13.4581 11.3685 13.6064C10.3813 14.1943 9.15795 14.4375 7.99836 14.4375C3.956 14.4374 1.28142 11.2234 1.54133 7.49805C1.77976 4.08387 4.81315 1.46094 8.25031 1.46094ZM6.12727 8.80176C6.27942 8.9613 6.43614 9.12597 6.56965 9.2666C6.67197 9.37438 6.76112 9.46823 6.82453 9.53516C6.856 9.56836 6.88138 9.59493 6.89875 9.61328L6.9261 9.6416C7.2937 10.0219 7.7023 10.154 8.10774 10.1201C8.52986 10.0848 8.998 9.86387 9.43293 9.44531C9.87269 9.02201 10.1181 8.56488 10.1722 8.14941C10.2235 7.75361 10.1099 7.34127 9.72492 6.94629L9.72102 6.94238L8.9261 6.10742L6.12727 8.80176Z" fill="currentColor"></path>'], // 原生「插件」
    "input": ['0 0 20 20', '<path d="M15.9951 7.33002C15.9951 6.61898 15.9942 6.12565 15.9629 5.74213C15.9398 5.46008 15.9021 5.27068 15.8506 5.12689L15.7949 4.99701C15.6409 4.69476 15.4068 4.44188 15.1191 4.26556L14.9932 4.19525C14.8352 4.11475 14.6237 4.05799 14.248 4.02728C13.8645 3.99595 13.3712 3.99506 12.6602 3.99506H7.33008C6.61904 3.99506 6.12571 3.99595 5.74219 4.02728C5.36648 4.05799 5.15508 4.11474 4.99707 4.19525C4.65183 4.37117 4.37124 4.65177 4.19531 4.99701C4.1148 5.15502 4.05805 5.36642 4.02734 5.74213C3.99601 6.12565 3.99512 6.61898 3.99512 7.33002V12.6601C3.99512 13.3711 3.99601 13.8645 4.02734 14.248C4.05805 14.6236 4.11481 14.8351 4.19531 14.9931L4.26563 15.1191C4.44194 15.4067 4.69482 15.6409 4.99707 15.7949L5.12695 15.8505C5.27074 15.902 5.46014 15.9398 5.74219 15.9628C6.12571 15.9942 6.61904 15.9951 7.33008 15.9951H12.6602C13.3712 15.9951 13.8645 15.9942 14.248 15.9628C14.6237 15.9321 14.8352 15.8754 14.9932 15.7949L15.1191 15.7245C15.4068 15.5482 15.6409 15.2953 15.7949 14.9931L15.8506 14.8632C15.9021 14.7194 15.9398 14.53 15.9629 14.248C15.9942 13.8645 15.9951 13.3711 15.9951 12.6601V7.33002ZM17.3252 12.6601C17.3252 13.3492 17.3258 13.9062 17.2891 14.3564C17.2563 14.757 17.1911 15.1178 17.0469 15.454L16.9805 15.5966C16.7149 16.1179 16.3105 16.5542 15.8145 16.8583L15.5967 16.9804C15.2201 17.1722 14.8142 17.2516 14.3564 17.289C13.9062 17.3258 13.3492 17.3251 12.6602 17.3251H7.33008C6.64099 17.3251 6.08403 17.3258 5.63379 17.289C5.23315 17.2563 4.87242 17.191 4.53613 17.0468L4.39356 16.9804C3.87229 16.7148 3.43595 16.3104 3.13184 15.8144L3.00977 15.5966C2.81795 15.22 2.73859 14.8142 2.70117 14.3564C2.66439 13.9062 2.66504 13.3492 2.66504 12.6601V7.33002C2.66504 6.64093 2.66439 6.08397 2.70117 5.63373C2.73858 5.17594 2.81796 4.77011 3.00977 4.39349C3.31321 3.79796 3.79802 3.31314 4.39356 3.00971C4.77017 2.8179 5.176 2.73852 5.63379 2.70111C6.08403 2.66433 6.64099 2.66498 7.33008 2.66498H12.6602C13.3492 2.66498 13.9062 2.66433 14.3564 2.70111C14.8142 2.73853 15.2201 2.81789 15.5967 3.00971L15.8145 3.13178C16.3105 3.43589 16.7149 3.87223 16.9805 4.39349L17.0469 4.53607C17.1911 4.87236 17.2563 5.23309 17.2891 5.63373C17.3258 6.08397 17.3252 6.64093 17.3252 7.33002V12.6601Z" fill="currentColor"></path><path d="M7.42654 10.6038L9.90146 6.85396C10.1554 6.46917 10.7538 6.7207 10.6565 7.17136L10.3159 8.75002H12.2258C12.5577 8.75002 12.7564 9.11918 12.5736 9.3962L10.0987 13.1461C9.8447 13.5309 9.24637 13.2793 9.34362 12.8287L9.68427 11.25H7.7743C7.44238 11.25 7.24371 10.8809 7.42654 10.6038Z" fill="currentColor"></path>'], // 原生「键盘快捷键」
    "chat": ['0 0 16 16', '<path d="M9.33423 8.14124C9.6238 8.14167 9.85864 8.37695 9.85864 8.66663C9.85864 8.9563 9.6238 9.19158 9.33423 9.19202H6.66724C6.37729 9.19202 6.14185 8.95658 6.14185 8.66663C6.14185 8.37668 6.37729 8.14124 6.66724 8.14124H9.33423Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M12.3333 2.14124C13.3595 2.14124 14.1915 2.97346 14.1917 3.99963V4.66663C14.1917 5.11035 14.0352 5.51699 13.7756 5.83655V11.3336C13.7755 12.7279 12.6445 13.8589 11.2502 13.859H4.75024C3.35584 13.859 2.22504 12.728 2.22485 11.3336V5.83752C1.96478 5.51781 1.80786 5.11082 1.80786 4.66663V3.99963C1.80804 2.97356 2.64019 2.14141 3.66626 2.14124H12.3333ZM12.7249 6.48206C12.5985 6.50918 12.4677 6.52502 12.3333 6.52502H3.66626C3.53214 6.525 3.4017 6.50907 3.27563 6.48206V11.3336C3.27582 12.1481 3.93574 12.8082 4.75024 12.8082H11.2502C12.0646 12.8081 12.7247 12.148 12.7249 11.3336V6.48206ZM3.66626 3.19202C3.22009 3.19219 2.85882 3.55346 2.85864 3.99963V4.66663C2.85864 4.94388 2.99856 5.18793 3.21118 5.33362H3.27563V5.37366C3.39145 5.43793 3.5244 5.47516 3.66626 5.47522H12.3333C12.4754 5.47522 12.6088 5.43811 12.7249 5.37366V5.33362H12.7903C13.0026 5.18786 13.1418 4.94346 13.1418 4.66663V3.99963C13.1417 3.55335 12.7796 3.19202 12.3333 3.19202H3.66626Z" fill="currentColor"></path>'], // 原生「已归档的聊天」
    "appearance": ['0 0 16 16', '<path d="M8 13.1406C8.28995 13.1406 8.52539 13.3761 8.52539 13.666V14.666C8.52539 14.956 8.28995 15.1914 8 15.1914C7.71016 15.1913 7.47461 14.9559 7.47461 14.666V13.666C7.47461 13.3761 7.71016 13.1408 8 13.1406Z" fill="currentColor"></path><path d="M11.7393 11.7324C11.9442 11.5276 12.2764 11.5268 12.4814 11.7314L13.0908 12.3418C13.2957 12.5468 13.2957 12.879 13.0908 13.084C12.8858 13.289 12.5527 13.289 12.3477 13.084L11.7383 12.4746C11.5336 12.2696 11.5344 11.9373 11.7393 11.7324Z" fill="currentColor"></path><path d="M3.52539 11.7305C3.73042 11.5258 4.06362 11.5256 4.26855 11.7305C4.47348 11.9354 4.47329 12.2686 4.26855 12.4736L3.6582 13.084C3.45313 13.2884 3.12085 13.2878 2.91602 13.083C2.71123 12.8782 2.71061 12.5459 2.91504 12.3408L3.52539 11.7305Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M8 4.14258C10.1307 4.14271 11.8582 5.87031 11.8584 8.00098C11.8584 10.1318 10.1308 11.8592 8 11.8594C5.86925 11.8592 4.1416 10.1318 4.1416 8.00098C4.14178 5.87034 5.86936 4.14275 8 4.14258ZM8 5.19336C6.44926 5.19354 5.19256 6.45023 5.19238 8.00098C5.19238 9.55187 6.44915 10.8094 8 10.8096C9.55089 10.8094 10.8086 9.5519 10.8086 8.00098C10.8084 6.45021 9.55078 5.19349 8 5.19336Z" fill="currentColor"></path><path d="M2.33691 7.47363C2.62681 7.47366 2.86225 7.70817 2.8623 7.99805C2.86228 8.28798 2.62684 8.52344 2.33691 8.52344H1.33691C1.04698 8.52341 0.811523 8.28797 0.811523 7.99805C0.811552 7.70811 1.04699 7.47266 1.33691 7.47266L2.33691 7.47363Z" fill="currentColor"></path><path d="M14.6699 7.47363C14.9598 7.47366 15.1953 7.70817 15.1953 7.99805C15.1953 8.28798 14.9598 8.52344 14.6699 8.52344H13.6699C13.3801 8.52328 13.1445 8.28789 13.1445 7.99805C13.1446 7.7082 13.3801 7.47279 13.6699 7.47266L14.6699 7.47363Z" fill="currentColor"></path><path d="M12.3438 2.91211C12.5488 2.70739 12.882 2.70723 13.0869 2.91211C13.2918 3.11704 13.2916 3.45022 13.0869 3.65527L12.4766 4.26562C12.2715 4.47007 11.9392 4.46948 11.7344 4.26465C11.5296 4.05981 11.529 3.72752 11.7334 3.52246L12.3438 2.91211Z" fill="currentColor"></path><path d="M2.9209 2.91211C3.12582 2.70728 3.45807 2.70648 3.66309 2.91113L4.27246 3.52148C4.47736 3.72648 4.47733 4.05867 4.27246 4.26367C4.06743 4.46866 3.7343 4.46868 3.5293 4.26367L2.91992 3.6543C2.71523 3.44924 2.71599 3.11702 2.9209 2.91211Z" fill="currentColor"></path><path d="M8 0.808594C8.28995 0.808594 8.52539 1.04403 8.52539 1.33398V2.33398C8.52539 2.62393 8.28995 2.85938 8 2.85938C7.71016 2.85924 7.47461 2.62385 7.47461 2.33398V1.33398C7.47461 1.04412 7.71016 0.808726 8 0.808594Z" fill="currentColor"></path>'], // 原生「外观」
    "pet": ['0 0 16 16', '<path d="M8.72925 8.8789V7.93135C8.72925 7.55641 9.03309 7.25256 9.40803 7.25256C9.78297 7.25256 10.0868 7.55641 10.0868 7.93135V8.8789C10.0867 9.25374 9.7829 9.55768 9.40803 9.55768C9.03317 9.55768 8.72937 9.25374 8.72925 8.8789Z" fill="currentColor"></path><path d="M5.91309 8.8789V7.93135C5.91309 7.55641 6.21693 7.25256 6.59187 7.25256C6.96681 7.25256 7.27066 7.55641 7.27066 7.93135V8.8789C7.27054 9.25374 6.96674 9.55768 6.59187 9.55768C6.21701 9.55768 5.9132 9.25374 5.91309 8.8789Z" fill="currentColor"></path><path d="M12.4746 9.26562C12.4745 7.40233 11.8081 5.60589 10.8838 4.2793C10.4223 3.61706 9.90605 3.08559 9.39453 2.72461C8.87891 2.36081 8.39952 2.19141 8 2.19141C7.60048 2.19141 7.12109 2.36081 6.60547 2.72461C6.09395 3.08559 5.57766 3.61706 5.11621 4.2793C4.19191 5.60589 3.52548 7.40233 3.52539 9.26562C3.52539 11.7651 5.26769 13.8076 8 13.8076C10.7323 13.8076 12.4746 11.7651 12.4746 9.26562ZM13.5254 9.26562C13.5254 12.2854 11.37 14.8574 8 14.8574C4.62996 14.8574 2.47461 12.2854 2.47461 9.26562C2.4747 7.16322 3.22245 5.16038 4.25488 3.67871C4.77191 2.93672 5.37038 2.3115 6 1.86719C6.62538 1.42587 7.31355 1.14062 8 1.14062C8.68645 1.14062 9.37462 1.42587 10 1.86719C10.6296 2.3115 11.2281 2.93672 11.7451 3.67871C12.7776 5.16038 13.5253 7.16322 13.5254 9.26562Z" fill="currentColor"></path>'], // 原生「迷你界面和智能伙伴」
    "stepwise": ['0 0 16 16', '<path d="M10.7939 8.15723C11.0751 8.22756 11.2459 8.51278 11.1758 8.79395C10.9829 9.56501 10.5871 10.1775 10.0176 10.5918C9.45181 11.0032 8.75339 11.1924 8 11.1924C7.71018 11.1924 7.47483 10.9568 7.47461 10.667C7.47461 10.377 7.71005 10.1416 8 10.1416C8.57975 10.1416 9.04857 9.99728 9.39941 9.74219C9.74618 9.48999 10.0167 9.10162 10.1572 8.54004C10.2275 8.2588 10.5127 8.087 10.7939 8.15723Z" fill="currentColor"></path><path d="M6.90039 5.03516C7.06472 4.79669 7.39211 4.7364 7.63086 4.90039C7.86962 5.06459 7.92958 5.39198 7.76562 5.63086L6.3916 7.62793L7.42773 7.81641C7.71256 7.86845 7.90108 8.14185 7.84961 8.42676C7.79774 8.71203 7.52453 8.90148 7.23926 8.84961L5.91309 8.6084C5.30046 8.49698 5.0066 7.79046 5.35938 7.27734L6.90039 5.03516Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M8 1.47461C11.6037 1.47461 14.5254 4.39634 14.5254 8C14.5254 11.6037 11.6037 14.5254 8 14.5254C4.39634 14.5254 1.47461 11.6037 1.47461 8C1.47461 4.39634 4.39634 1.47461 8 1.47461ZM8 2.52539C4.97624 2.52539 2.52539 4.97624 2.52539 8C2.52539 11.0238 4.97624 13.4746 8 13.4746C11.0238 13.4746 13.4746 11.0238 13.4746 8C13.4746 4.97624 11.0238 2.52539 8 2.52539Z" fill="currentColor"></path>'], // 原生「个性化」
    "standalone": ['0 0 16 16', '<path d="M5.13802 4.65365C5.51357 4.65365 5.81771 4.95778 5.81771 5.33333C5.8177 5.70888 5.51357 6.01302 5.13802 6.01302C4.76258 6.0129 4.45834 5.7088 4.45833 5.33333C4.45833 4.95786 4.76257 4.65377 5.13802 4.65365Z" fill="currentColor"></path><path d="M10.6667 4.79948C10.9612 4.79948 11.2005 5.03878 11.2005 5.33333C11.2005 5.62789 10.9612 5.86719 10.6667 5.86719H8C7.70545 5.86719 7.46615 5.62789 7.46615 5.33333C7.46615 5.03878 7.70545 4.79948 8 4.79948H10.6667Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M10.0664 2.13281C10.8043 2.13281 11.3906 2.13336 11.862 2.17188C12.3394 2.21092 12.747 2.29181 13.1198 2.48177C13.7218 2.78855 14.2115 3.2782 14.5182 3.88021C14.7082 4.25302 14.7891 4.66056 14.8281 5.13802C14.8666 5.60945 14.8672 6.19566 14.8672 6.93359V9.06641C14.8672 9.80434 14.8666 10.3906 14.8281 10.862C14.7891 11.3394 14.7082 11.747 14.5182 12.1198C14.2115 12.7218 13.7218 13.2115 13.1198 13.5182C12.747 13.7082 12.3394 13.7891 11.862 13.8281C11.3906 13.8666 10.8043 13.8672 10.0664 13.8672H5.93359C5.19566 13.8672 4.60945 13.8666 4.13802 13.8281C3.66056 13.7891 3.25302 13.7082 2.88021 13.5182C2.2782 13.2115 1.78855 12.7218 1.48177 12.1198C1.29181 11.747 1.21092 11.3394 1.17188 10.862C1.13336 10.3906 1.13281 9.80434 1.13281 9.06641V6.93359C1.13281 6.19566 1.13336 5.60945 1.17188 5.13802C1.21092 4.66056 1.29181 4.25302 1.48177 3.88021C1.78855 3.2782 2.2782 2.78855 2.88021 2.48177C3.25302 2.29181 3.66056 2.21092 4.13802 2.17188C4.60945 2.13336 5.19566 2.13281 5.93359 2.13281H10.0664ZM5.93359 3.20052C5.17806 3.20052 4.64318 3.20023 4.22526 3.23438C3.81359 3.26801 3.56208 3.33166 3.36458 3.43229C2.96325 3.63681 2.63681 3.96325 2.43229 4.36458C2.33166 4.56208 2.26801 4.81359 2.23438 5.22526C2.20023 5.64318 2.20052 6.17806 2.20052 6.93359V9.06641C2.20052 9.82194 2.20023 10.3568 2.23438 10.7747C2.26801 11.1864 2.33166 11.4379 2.43229 11.6354C2.63681 12.0368 2.96325 12.3632 3.36458 12.5677C3.56208 12.6683 3.81359 12.732 4.22526 12.7656C4.64318 12.7998 5.17806 12.7995 5.93359 12.7995H10.0664C10.8219 12.7995 11.3568 12.7998 11.7747 12.7656C12.1864 12.732 12.4379 12.6683 12.6354 12.5677C13.0368 12.3632 13.3632 12.0368 13.5677 11.6354C13.6683 11.4379 13.732 11.1864 13.7656 10.7747C13.7998 10.3568 13.7995 9.82194 13.7995 9.06641V6.93359C13.7995 6.17806 13.7998 5.64318 13.7656 5.22526C13.732 4.81359 13.6683 4.56208 13.5677 4.36458C13.3632 3.96325 13.0368 3.63681 12.6354 3.43229C12.4379 3.33166 12.1864 3.26801 11.7747 3.23438C11.3568 3.20023 10.8219 3.20052 10.0664 3.20052H5.93359Z" fill="currentColor"></path>'], // 原生「浏览器」
    "tools": ['0 0 16 16', '<path d="M10.666 9.6416C10.956 9.6416 11.1914 9.87704 11.1914 10.167C11.1913 10.4568 10.9559 10.6924 10.666 10.6924H5.33301C5.04325 10.6923 4.80775 10.4567 4.80762 10.167C4.80762 9.87712 5.04317 9.64173 5.33301 9.6416H10.666Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M12.1338 2.47461C13.4543 2.47486 14.5251 3.54569 14.5254 4.86621V11.1338C14.5251 12.4543 13.4543 13.5251 12.1338 13.5254H3.86621C2.54569 13.5251 1.47486 12.4543 1.47461 11.1338V4.86621C1.47486 3.54569 2.54569 2.47486 3.86621 2.47461H12.1338ZM3.86621 3.52539C3.12559 3.52564 2.52564 4.12559 2.52539 4.86621V11.1338C2.52564 11.8744 3.12559 12.4744 3.86621 12.4746H12.1338C12.8744 12.4744 13.4744 11.8744 13.4746 11.1338V4.86621C13.4744 4.12559 12.8744 3.52564 12.1338 3.52539H3.86621Z" fill="currentColor"></path>'], // 原生「环境」
    "ext-menu": ['0 0 16 16', '<path d="M1.66699 9.53271C1.95694 9.53271 2.19238 9.76816 2.19238 10.0581V10.1333C2.19238 10.7019 2.19268 11.0977 2.21777 11.4048C2.24238 11.7054 2.28759 11.876 2.35254 12.0034C2.49394 12.2808 2.71971 12.5066 2.99707 12.6479C3.12456 12.7129 3.29503 12.7581 3.5957 12.7827C3.90287 12.8078 4.29849 12.8081 4.86719 12.8081H5C5.28995 12.8081 5.52539 13.0435 5.52539 13.3335C5.52526 13.6233 5.28987 13.8589 5 13.8589H4.86719C4.31586 13.8589 3.87056 13.859 3.51074 13.8296C3.14481 13.7997 2.82074 13.7364 2.52051 13.5835C2.0455 13.3415 1.65907 12.955 1.41699 12.48C1.26404 12.1798 1.20081 11.8556 1.1709 11.4897C1.14151 11.13 1.1416 10.6845 1.1416 10.1333V10.0581C1.1416 9.76816 1.37704 9.53271 1.66699 9.53271Z" fill="currentColor"></path><path d="M14.333 9.4751C14.623 9.4751 14.8584 9.71054 14.8584 10.0005V10.1333C14.8584 10.6845 14.8585 11.13 14.8291 11.4897C14.7992 11.8556 14.736 12.1798 14.583 12.48C14.3409 12.9549 13.9545 13.3415 13.4795 13.5835C13.1793 13.7364 12.8552 13.7997 12.4893 13.8296C12.1294 13.859 11.6841 13.8589 11.1328 13.8589H10.833C10.5432 13.8589 10.3078 13.6233 10.3076 13.3335C10.3076 13.0435 10.5431 12.8081 10.833 12.8081H11.1328C11.7015 12.8081 12.0971 12.8078 12.4043 12.7827C12.705 12.7581 12.8754 12.7129 13.0029 12.6479C13.2803 12.5066 13.5061 12.2807 13.6475 12.0034C13.7124 11.876 13.7576 11.7053 13.7822 11.4048C13.8073 11.0977 13.8076 10.7019 13.8076 10.1333V10.0005C13.8076 9.71054 14.0431 9.4751 14.333 9.4751Z" fill="currentColor"></path><path d="M5 2.14111C5.28995 2.14111 5.52539 2.37655 5.52539 2.6665C5.52539 2.95645 5.28995 3.19189 5 3.19189H4.86719C4.29847 3.19189 3.90287 3.19219 3.5957 3.21729C3.29495 3.24189 3.12457 3.28709 2.99707 3.35205C2.71964 3.49344 2.49393 3.71915 2.35254 3.99658C2.28757 4.12408 2.24238 4.29447 2.21777 4.59521C2.19268 4.90238 2.19238 5.29798 2.19238 5.8667V5.99951C2.19238 6.28946 1.95694 6.5249 1.66699 6.5249C1.37704 6.5249 1.1416 6.28946 1.1416 5.99951V5.8667C1.1416 5.31536 1.14151 4.87008 1.1709 4.51025C1.2008 4.14428 1.264 3.82028 1.41699 3.52002C1.65907 3.04494 2.04543 2.65858 2.52051 2.4165C2.82077 2.26352 3.14477 2.20031 3.51074 2.17041C3.87057 2.14102 4.31585 2.14111 4.86719 2.14111H5Z" fill="currentColor"></path><path d="M11.1338 2.14111C11.6847 2.14111 12.1296 2.14105 12.4893 2.17041C12.8551 2.2003 13.1793 2.26361 13.4795 2.4165C13.9545 2.65855 14.3409 3.045 14.583 3.52002C14.736 3.82028 14.7992 4.14428 14.8291 4.51025C14.8585 4.8701 14.8584 5.31531 14.8584 5.8667V5.99951C14.8584 6.28946 14.623 6.52491 14.333 6.5249C14.0432 6.52473 13.8086 6.28936 13.8086 5.99951V5.8667C13.8086 5.29797 13.8083 4.90239 13.7832 4.59521C13.7586 4.29445 13.7124 4.12409 13.6475 3.99658C13.506 3.71918 13.2804 3.49342 13.0029 3.35205C12.8754 3.2871 12.7052 3.24187 12.4043 3.21729C12.0972 3.19219 11.7023 3.19189 11.1338 3.19189H11C10.7101 3.19189 10.4746 2.95645 10.4746 2.6665C10.4746 2.37655 10.7101 2.14111 11 2.14111H11.1338Z" fill="currentColor"></path><path d="M4.38672 4.55615C4.7326 4.55631 5.01255 4.83625 5.0127 5.18213C5.0127 5.52813 4.73269 5.80893 4.38672 5.80908C4.04062 5.80908 3.75977 5.52823 3.75977 5.18213C3.75991 4.83615 4.04071 4.55615 4.38672 4.55615Z" fill="currentColor"></path><path d="M6.69238 4.55615C7.03839 4.55615 7.31919 4.83615 7.31934 5.18213C7.31934 5.52823 7.03848 5.80908 6.69238 5.80908C6.34638 5.80897 6.06641 5.52816 6.06641 5.18213C6.06655 4.83622 6.34646 4.55626 6.69238 4.55615Z" fill="currentColor"></path><path d="M8.99902 4.55615C9.3449 4.55631 9.62486 4.83625 9.625 5.18213C9.625 5.52813 9.34499 5.80893 8.99902 5.80908C8.65293 5.80908 8.37207 5.52823 8.37207 5.18213C8.37221 4.83615 8.65301 4.55615 8.99902 4.55615Z" fill="currentColor"></path>'], // 原生「应用快照」
    // 拓展注册的设置分区（registerSettingsSection）：内置的两个用原生对应页的图标，其余用「插件」。
    "hooks-editor": ['0 0 16 16', '<path fill-rule="evenodd" clip-rule="evenodd" d="M8 1.59766C9.39452 1.59766 10.5254 2.72853 10.5254 4.12305C10.5254 5.33728 9.66764 6.34981 8.52539 6.5918V11.0342C8.52556 12.2139 9.47942 13.1717 10.6592 13.1758C11.8447 13.1798 12.8084 12.2197 12.8086 11.0342V9.95215L11.8086 10.6797C11.5742 10.8498 11.2457 10.7977 11.0752 10.5635C10.9049 10.329 10.9571 10.0006 11.1914 9.83008L12.7324 8.70898C13.2008 8.36838 13.8581 8.70314 13.8584 9.28223V11.0342C13.8582 12.801 12.4221 14.2316 10.6553 14.2256C9.5416 14.2216 8.56418 13.6464 7.99609 12.7793C7.42476 13.6433 6.44623 14.2148 5.33301 14.2148C3.57068 14.2147 2.14197 12.7857 2.1416 11.0234V9.28223C2.1416 8.70306 2.79918 8.36833 3.26758 8.70898L4.80859 9.83008C5.04289 10.0006 5.09511 10.329 4.9248 10.5635C4.75431 10.7977 4.42581 10.8498 4.19141 10.6797L3.19141 9.95215V11.0234C3.19178 12.2058 4.15058 13.1649 5.33301 13.165C6.51559 13.165 7.47424 12.2059 7.47461 11.0234V6.5918C6.33236 6.34981 5.47461 5.33728 5.47461 4.12305C5.47461 2.72853 6.60548 1.59766 8 1.59766ZM8 2.64844C7.18538 2.64844 6.52539 3.30843 6.52539 4.12305C6.52539 4.93767 7.18538 5.59766 8 5.59766C8.81462 5.59766 9.47461 4.93767 9.47461 4.12305C9.47461 3.30843 8.81462 2.64844 8 2.64844Z" fill="currentColor"></path>'], // 原生「钩子」
    "feature-lab": ['0 0 16 16', '<path d="M10.7639 7.975C11.0539 7.975 11.2893 8.21044 11.2893 8.50039C11.2891 8.79017 11.0537 9.02578 10.7639 9.02578H8.69751C8.40779 9.02565 8.17232 8.79009 8.17212 8.50039C8.17212 8.21052 8.40767 7.97513 8.69751 7.975H10.7639Z" fill="currentColor"></path><path d="M5.02661 5.46231C5.23163 5.25729 5.56377 5.2573 5.7688 5.46231L6.98462 6.67813C7.25433 6.94821 7.25444 7.38565 6.98462 7.65567L5.7688 8.87149C5.56381 9.07623 5.23158 9.07627 5.02661 8.87149C4.82168 8.66653 4.82182 8.33433 5.02661 8.1293L5.98853 7.16641L5.02661 6.20449C4.82163 5.99949 4.82165 5.66733 5.02661 5.46231Z" fill="currentColor"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M7.2561 1.13418C7.73078 0.92667 8.2717 0.926585 8.74634 1.13418L12.7405 2.88125C13.4166 3.17717 13.8536 3.84633 13.8538 4.58438V8.38125C13.8538 10.1831 13.2191 11.5903 12.2014 12.6684C11.1953 13.7339 9.83514 14.4574 8.40356 14.9564L8.39966 14.9574C8.14413 15.044 7.86712 15.041 7.61353 14.9496C6.17841 14.454 4.81353 13.7338 3.80396 12.6693C2.78265 11.5924 2.14575 10.1849 2.14575 8.38125V4.58438C2.14594 3.84613 2.58358 3.17705 3.26001 2.88125L7.2561 1.13418ZM8.32544 2.0961C8.11904 2.00585 7.88342 2.00589 7.677 2.0961L3.68091 3.84414C3.38676 3.97277 3.19672 4.26336 3.19653 4.58438V8.38125C3.19653 9.91061 3.72656 11.0608 4.56665 11.9467C5.31218 12.7326 6.31853 13.3274 7.45923 13.7748L7.9563 13.9574L7.96802 13.9613C7.99845 13.9726 8.03201 13.9737 8.06274 13.9633C9.40244 13.4958 10.5895 12.8452 11.4377 11.9467C12.2751 11.0596 12.804 9.90981 12.804 8.38125V4.58438C12.8038 4.26348 12.6136 3.97187 12.3196 3.84317L8.32544 2.0961Z" fill="currentColor"></path>'], // 原生「配置」
    "ext-permissions": ['0 0 20 20', '<path fill-rule="evenodd" clip-rule="evenodd" d="M6.99512 2.7002C7.44919 2.7002 7.89367 2.83504 8.27148 3.08691L9.93164 4.19336C10.1014 4.30652 10.3009 4.36719 10.5049 4.36719H15.2998C16.8645 4.36719 18.1335 5.6356 18.1338 7.2002V14.334C18.1338 15.9723 16.8053 17.3006 15.167 17.3008H4.83301C3.19471 17.3006 1.86621 15.9723 1.86621 14.334V5.66699C1.86639 4.0288 3.19482 2.70037 4.83301 2.7002H6.99512ZM16.8662 9.80078H3.13379V14.334C3.13379 15.2728 3.89427 16.034 4.83301 16.0342H15.167C16.1057 16.034 16.8662 15.2728 16.8662 14.334V9.80078ZM3.13379 8.5332H16.8662V7.2002C16.866 6.33516 16.1649 5.63379 15.2998 5.63379H10.5049C10.0508 5.63379 9.60633 5.49992 9.22852 5.24805L7.56836 4.14062C7.39867 4.02757 7.19902 3.96777 6.99512 3.96777H4.83301C3.89438 3.96795 3.13396 4.72837 3.13379 5.66699V8.5332Z" fill="currentColor"></path>'], // 原生「存储」
  };

  function codexPlusSettingsGroupIcon(id) {
    const [viewBox, glyph] = codexPlusSettingsGroupIcons[id] || codexPlusSettingsGroupIcons.core;
    return `<svg viewBox="${viewBox}" width="16" height="16" fill="none">${glyph}</svg>`;
  }

  /**
   * 左栏分区小标题，照原生设置页的「个人 / 集成 / 编码」。分组按 DOM 顺序排，换区时插一条小标题；
   * 拓展注册的分区与「拓展设置」归「拓展」，清单里新加、没登记的分组归「功能」。
   */
  const codexPlusSettingsNavSections = {
    core: "通用", plugins: "通用", input: "通用", chat: "通用", appearance: "通用",
    pet: "功能", stepwise: "功能", standalone: "功能",
    "ext-permissions": "拓展", "ext-menu": "拓展",
    tools: "其他",
  };

  function codexPlusSettingsNavSection(id) {
    if (String(id).startsWith("ext-section:")) return "拓展";
    return codexPlusSettingsNavSections[id] || "功能";
  }

  /**
   * 开着的那一页自己的分组（远程控制、微信连接，installCodexPlusEmbeddedNav 设置）：{ entry, nav, active }。
   * 分组挂在左栏那一项下面，右边只显示选中的一组。
   */
  let codexPlusEmbeddedSection = null;

  /**
   * 左栏最上面的「页面」一节：收进 Codex++ 入口的页面，点一项打开那一页。
   * 设置页和图标栏悬浮卡片共用；当前开着的那一页标成选中（设置页本身开着时一项都不选）。
   * 开着的那一页有自己的分组时，分组缩进列在它下面，选中的是分组，页面那一项不再整行高亮。
   */
  function codexPlusFoldedPagesNavHtml() {
    const current = codexPlusActiveEntry();
    return `
      <div class="codex-plus-page-nav-section-title" role="presentation">页面</div>${codexPlusFoldedPages().map((page) => {
      const section = page.entry === current && codexPlusEmbeddedSection?.entry === page.entry ? codexPlusEmbeddedSection : null;
      const on = page.entry === current && !section;
      const children = section
        ? `
      <div class="codex-plus-page-nav-children" role="group" aria-label="${escapeHtml(page.label)}分组">${section.nav.groups.map((group) => {
          const active = group.id === section.active;
          return `<button type="button" class="codex-plus-page-nav-item codex-plus-page-nav-subitem" data-codex-plus-embedded-section="${escapeHtml(group.id)}" data-active="${String(active)}"${active ? ' aria-current="page"' : ""}><span class="codex-plus-page-nav-item-name">${escapeHtml(group.label)}</span></button>`;
        }).join("")}</div>`
        : "";
      return `
      <button type="button" class="codex-plus-page-nav-item" data-codex-plus-page-link="${escapeHtml(page.entry)}" data-active="${String(on)}"${on ? ' aria-current="page"' : ""}${section ? ' data-expanded="true"' : ""}><span class="codex-plus-page-nav-item-icon" aria-hidden="true">${page.icon}</span><span class="codex-plus-page-nav-item-name">${escapeHtml(page.label)}${page.tagHtml || ""}</span></button>${children}`;
    }).join("")}`;
  }

  function renderCodexPlusSettingsNavItems(root = document) {
    const groups = codexPlusSettingsGroupNodes(root);
    const active = codexPlusResolvedSettingsGroup(groups);
    let section = "";
    return codexPlusFoldedPagesNavHtml() + groups.map((node) => {
      const id = node.getAttribute("data-codex-plus-settings-group");
      const label = node.getAttribute("data-codex-plus-settings-label") || id;
      const current = id === active;
      const nextSection = codexPlusSettingsNavSection(id);
      const title = nextSection !== section
        ? `
      <div class="codex-plus-page-nav-section-title" role="presentation">${escapeHtml(nextSection)}</div>`
        : "";
      section = nextSection;
      return `${title}
      <button type="button" class="codex-plus-page-nav-item" data-codex-plus-settings-nav="${escapeHtml(id)}" data-active="${String(current)}"${current ? ' aria-current="page"' : ""}><span class="codex-plus-page-nav-item-icon" aria-hidden="true">${codexPlusSettingsGroupIcon(node.getAttribute("data-codex-plus-settings-icon") || id)}</span><span class="codex-plus-page-nav-item-name">${escapeHtml(label)}</span></button>`;
    }).join("");
  }

  /** 把当前分组写回页面：分组的 data-active、左栏选中态。重建分组或左栏后都要调一次。 */
  function syncCodexPlusSettingsGroups(root = document) {
    const active = codexPlusResolvedSettingsGroup(codexPlusSettingsGroupNodes(root));
    root.querySelectorAll('[data-codex-plus-panel="home"] > [data-codex-plus-settings-group]').forEach((node) => {
      const value = String(node.getAttribute("data-codex-plus-settings-group") === active);
      if (node.dataset.active !== value) node.dataset.active = value;
      if (node.getAttribute("data-codex-plus-settings-group") === "ext-permissions") {
        const details = node.querySelector("[data-codex-plus-usage-storage]");
        const visible = value === "true" || !node.closest('[data-codex-plus-settings-paged="true"]');
        if (details && details.dataset.active !== String(visible)) {
          details.dataset.active = String(visible);
          if (visible && !details.dataset.loading) void loadCodexPlusUsageStorage(details);
        }
      }
    });
    root.querySelectorAll("[data-codex-plus-settings-nav]").forEach((button) => {
      const current = button.getAttribute("data-codex-plus-settings-nav") === active;
      if (button.dataset.active !== String(current)) button.dataset.active = String(current);
      if (current) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    });
    syncCodexPlusSettingsSectionMounts(root);
  }

  function selectCodexPlusSettingsGroup(id) {
    codexPlusSettingsActiveGroup = String(id || "");
    try { window.localStorage.setItem(codexPlusSettingsGroupStorageKey, codexPlusSettingsActiveGroup); } catch {}
    syncCodexPlusSettingsGroups();
    // 换一组从顶上看起。
    const body = document.querySelector('[data-codex-plus-panel="home"]')?.closest(".codex-plus-modal-body");
    if (body) body.scrollTop = 0;
  }

  /** 分组列表变了（清单读完、拓展菜单增减）时重画左栏；不在设置页时左栏属于别的页面，不动。 */
  function refreshCodexPlusSettingsNav() {
    if (document.querySelector(".codex-plus-modal-content")?.dataset.codexPlusActiveTab !== "home") return;
    const body = document.querySelector("[data-codex-plus-page-nav-body]");
    if (body) body.innerHTML = renderCodexPlusSettingsNavItems();
    syncCodexPlusSettingsGroups();
  }

  // ── 拓展注册的设置分区（registerSettingsSection，92-extension-views.js 登记）────────────
  // 一个分区一组，排在清单分组之后、「工具与关于」之前。外壳（标题 + 挂载点）随设置页重建；
  // 内容只在选中时调 render 画，切走或关页时调清理函数并清空——注册表持久、DOM 瞬态。
  const codexPlusSettingsSectionGroupPrefix = "ext-section:";

  function codexPlusSettingsSectionItems() {
    return codexPlusExtensionItems(codexPlusRegistry.settingsSections);
  }

  /** 分区列表的指纹：没变就不动 DOM，已经画好的分区（比如编辑到一半的 Hooks）不会被别的拓展注册冲掉。 */
  function codexPlusSettingsSectionsFingerprint() {
    return codexPlusSettingsSectionItems().map((item) => `${item.id}\u0000${item.title || ""}`).join("\u0001");
  }

  function renderCodexPlusSettingsSectionGroups() {
    return codexPlusSettingsSectionItems().map((item) => {
      const title = String(item.title || item.sectionKey || item.id);
      const icon = Object.prototype.hasOwnProperty.call(codexPlusSettingsGroupIcons, item.sectionKey) ? item.sectionKey : "plugins";
      return renderCodexPlusSettingsGroup(`${codexPlusSettingsSectionGroupPrefix}${item.id}`, title, `
            <section class="codex-plus-settings-section" aria-label="${escapeHtml(title)}">
              <h2 class="codex-plus-settings-section-title">${escapeHtml(title)}</h2>
              <div class="codex-plus-settings-extension-section" data-codex-plus-settings-section-mount="${escapeHtml(item.id)}" ${codexPlusExtensionConstants.extensionAttribute}="${escapeHtml(item.scriptKey || "")}"></div>
            </section>`, ` data-codex-plus-settings-extension="true" data-codex-plus-settings-icon="${escapeHtml(icon)}"`);
    }).join("");
  }

  function mountCodexPlusSettingsSection(mount) {
    mount.__codexPlusSectionMounted = true;
    const item = codexPlusRegistry.settingsSections.get(mount.getAttribute("data-codex-plus-settings-section-mount"));
    if (!item) return;
    const outcome = runCodexPlusExtensionCallback(item.scriptKey, "settingsSection.render", () =>
      item.render({ container: mount, close: () => closeCodexPlusPage(), script: item.scriptKey }));
    if (outcome.ok && typeof outcome.value === "function") mount.__codexPlusSectionCleanup = outcome.value;
    if (!outcome.ok) {
      mount.innerHTML = `
              <div class="codex-plus-settings-card"><div class="codex-plus-row"><div><div class="codex-plus-row-title">拓展分区加载失败</div><div class="codex-plus-row-description">${escapeHtml(item.scriptKey || "")}：${escapeHtml(outcome.error)}</div></div></div></div>`;
    }
  }

  function unmountCodexPlusSettingsSection(mount) {
    const cleanup = mount.__codexPlusSectionCleanup;
    mount.__codexPlusSectionCleanup = null;
    mount.__codexPlusSectionMounted = false;
    if (typeof cleanup === "function") {
      const item = codexPlusRegistry.settingsSections.get(mount.getAttribute("data-codex-plus-settings-section-mount"));
      runCodexPlusExtensionCallback(item?.scriptKey || "", "settingsSection.cleanup", cleanup);
    }
    mount.replaceChildren();
  }

  /** 分页时只画当前选中的那组；不分页（弹窗）时每组都显示，全都画。 */
  function syncCodexPlusSettingsSectionMounts(root = document) {
    root.querySelectorAll("[data-codex-plus-settings-section-mount]").forEach((mount) => {
      const group = mount.closest("[data-codex-plus-settings-group]");
      const paged = !!mount.closest('[data-codex-plus-settings-paged="true"]');
      const visible = !paged || group?.getAttribute("data-active") === "true";
      if (visible && !mount.__codexPlusSectionMounted) mountCodexPlusSettingsSection(mount);
      else if (!visible && mount.__codexPlusSectionMounted) unmountCodexPlusSettingsSection(mount);
    });
  }

  /** 关页前把画着的分区都收掉，让拓展的清理函数有机会跑。 */
  function unmountCodexPlusSettingsSections(root = document) {
    root.querySelectorAll("[data-codex-plus-settings-section-mount]").forEach((mount) => {
      if (mount.__codexPlusSectionMounted) unmountCodexPlusSettingsSection(mount);
    });
  }

  /** 注册 / 撤销分区后重建分区外壳（指纹没变就跳过），再重画左栏。 */
  function refreshCodexPlusSettingsSections() {
    const fingerprint = codexPlusSettingsSectionsFingerprint();
    let changed = false;
    document.querySelectorAll('[data-codex-plus-panel="home"]').forEach((panel) => {
      if (panel.getAttribute("data-codex-plus-settings-sections") === fingerprint) return;
      const anchor = panel.querySelector('[data-codex-plus-settings-group="ext-menu"], [data-codex-plus-settings-tools]');
      if (!anchor) return;
      panel.querySelectorAll("[data-codex-plus-settings-extension]").forEach((node) => {
        unmountCodexPlusSettingsSections(node);
        node.remove();
      });
      anchor.insertAdjacentHTML("beforebegin", renderCodexPlusSettingsSectionGroups());
      panel.setAttribute("data-codex-plus-settings-sections", fingerprint);
      changed = true;
    });
    if (changed) refreshCodexPlusSettingsNav();
  }

  function normalizeCodexPlusSettingsCatalog(value) {
    if (!value || typeof value !== "object" || !Array.isArray(value.groups) || !Array.isArray(value.items)) return null;
    const groups = value.groups.filter((group) => group && typeof group.id === "string" && typeof group.title === "string");
    const items = value.items.filter((item) => item && typeof item.key === "string" && typeof item.group === "string"
      && typeof item.title === "string" && Array.isArray(item.surfaces));
    const builtins = (Array.isArray(value.builtins) ? value.builtins : []).filter((entry) => entry && typeof entry.id === "string"
      && typeof entry.group === "string" && typeof entry.title === "string");
    return groups.length && items.length ? { groups, items, builtins } : null;
  }

  function codexPlusSettingsCatalogItemVisible(item) {
    const platform = codexPlusIsWindowsPlatform ? "windows" : codexPlusIsMacPlatform ? "macos" : "linux";
    return item.surfaces.includes("codex") && (!Array.isArray(item.platforms) || item.platforms.includes(platform));
  }

  /** 读一次清单并缓存；失败后由设置页上的「重试」再次触发。 */
  function loadCodexPlusSettingsCatalog() {
    if (codexPlusSettingsCatalog) return Promise.resolve(codexPlusSettingsCatalog);
    if (codexPlusSettingsCatalogRequest) return codexPlusSettingsCatalogRequest;
    const retrying = codexPlusSettingsCatalogStatus.status === "failed";
    codexPlusSettingsCatalogStatus = { status: "loading", message: "" };
    if (retrying) refreshCodexPlusSettingsCatalogView();
    codexPlusSettingsCatalogRequest = Promise.resolve()
      .then(() => postJson("/settings/catalog", {}))
      .then((result) => {
        const catalog = normalizeCodexPlusSettingsCatalog(result);
        if (catalog) {
          codexPlusSettingsCatalog = catalog;
          codexPlusSettingsCatalogStatus = { status: "ok", message: "" };
          return;
        }
        const message = result?.status === "failed" && result.message ? String(result.message) : "后端返回的设置项清单格式不正确";
        codexPlusSettingsCatalogStatus = { status: "failed", message };
      }, (error) => {
        codexPlusSettingsCatalogStatus = { status: "failed", message: error?.message || String(error) };
      })
      .then(() => {
        codexPlusSettingsCatalogRequest = null;
        if (codexPlusSettingsCatalogStatus.status === "failed") {
          sendCodexPlusDiagnostic("settings_catalog_load_failed", { message: codexPlusSettingsCatalogStatus.message });
        }
        refreshCodexPlusSettingsCatalogView();
        return codexPlusSettingsCatalog;
      });
    return codexPlusSettingsCatalogRequest;
  }

  function renderCodexPlusSettingsSection(title, rows, description = "") {
    const label = escapeHtml(title);
    return `
            <section class="codex-plus-settings-section" aria-label="${label}" data-codex-plus-settings-catalog="section">
              <h2 class="codex-plus-settings-section-title">${label}</h2>${description ? `
              <p class="codex-plus-settings-section-description">${escapeHtml(description)}</p>` : ""}
              <div class="codex-plus-settings-card">${rows}</div>
            </section>`;
  }

  /** 一个分组：清单分组（连同接在后面的服务配置节）或「工具与关于」，左栏里一项对应一个。 */
  function renderCodexPlusSettingsGroup(id, label, content, attrs = "") {
    return `
          <div class="codex-plus-settings-group" data-codex-plus-settings-group="${escapeHtml(id)}" data-codex-plus-settings-label="${escapeHtml(label)}" data-active="${String(id === codexPlusSettingsActiveGroup)}"${attrs}>${content}
          </div>`;
  }

  /** 后端连接状态不是设置项，固定放在第一组的最前面（清单读取失败时也要能看到它）。 */
  function renderCodexPlusBackendStatusRow() {
    return `
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">后端连接</div><div class="codex-plus-row-description">每 5 秒检查一次 launcher 后端状态。</div></div>
                  <div class="codex-plus-backend-status">
                    <div class="codex-plus-backend-label" data-codex-backend-status="true" data-status="checking">正在检查后端…</div>
                  </div>
                </div>`;
  }

  let codexPlusUsageStorageScale;

  /**
   * 只在 Codex 设置页出现的行内输入框。用量保留策略保存在后端。
   * 清单只给读屏标签与悬停提示；取值范围、当前值仍由运行时常量和读写函数决定。
   */
  function renderCodexPlusSettingsCatalogInput(input) {
    const storageMiB = codexPlusBackendSettings.usageHistoryStorageMiB ?? 2048;
    const scale = codexPlusUsageStorageScale ?? (storageMiB >= 1024 ? 1024 : 1);
    const specs = {
      usageHistoryStorageMiB: () => ({ attribute: "data-codex-plus-usage-history-budget", min: 1 / scale, max: 4294967295 / scale, step: 1 / scale, value: storageMiB / scale, unit: "storage" }),
      usageHistoryPreciseDays: () => ({ attribute: "data-codex-plus-usage-history-days", min: 1, max: 365, step: 1, value: codexPlusBackendSettings.usageHistoryPreciseDays ?? 60, unit: "天" }),
      capacityAutoRetryIntervalSeconds: () => ({ attribute: "data-codex-plus-capacity-retry-interval", min: 0, max: capacityAutoRetryIntervalCeilingSeconds, step: 1, value: capacityAutoRetryIntervalSeconds() }),
      capacityAutoRetryMaxAttempts: () => ({ attribute: "data-codex-plus-capacity-retry-max-attempts", min: 1, max: capacityAutoRetryMaxAttemptsCeiling, step: 1, value: capacityAutoRetryMaxAttempts() }),
      conversationViewMaxWidth: () => ({ attribute: "data-codex-plus-conversation-view-width", min: conversationViewMinWidth, max: conversationViewMaxAllowedWidth, step: 10, value: conversationViewWidth() }),
    };
    const spec = specs[input?.id];
    if (!spec) return "";
    const { attribute, min, max, step, value, unit } = spec();
    const label = escapeHtml(input.label || "");
    const policy = unit ? ` data-codex-plus-usage-policy="${input.id}"` : "";
    const disabled = unit && codexPlusBackendSettings.usageHistoryEnabled !== true ? " disabled" : "";
    const control = `<input class="codex-plus-width-input" ${attribute}="true"${policy} min="${min}" max="${max}" step="${step}" type="number" value="${value}" title="${escapeHtml(input.hint || input.label || "")}" aria-label="${label}"${disabled}>`;
    if (unit === "storage") return `${control}<select class="codex-plus-width-input codex-plus-usage-unit" data-codex-plus-usage-unit aria-label="存储上限单位" title="1 G = 1024 M"${disabled}><option value="1"${scale === 1 ? " selected" : ""}>M</option><option value="1024"${scale === 1024 ? " selected" : ""}>G</option></select>`;
    return unit ? `${control}<span>天</span>` : control;
  }

  /** 清单 key 是 BackendSettings 字段；能映射到本页设置 key 的走 setCodexPlusSetting（带即时副作用）。 */
  function codexPlusLocalSettingKey(backendKey) {
    return Object.keys(codexPlusBackendSettingMap).find((key) => codexPlusBackendSettingMap[key] === backendKey) || "";
  }

  function codexPlusSettingsCatalogDescription(item) {
    const enhancementsOn = codexPlusBackendSettings.enhancementsEnabled !== false;
    return (enhancementsOn && item.descriptionWhenEnhancementsOn) || item.description || "";
  }

  /** 总开关变化后刷新带两套说明的行。 */
  function syncCodexPlusSettingsCatalogDescriptions() {
    if (!codexPlusSettingsCatalog) return;
    document.querySelectorAll("[data-codex-plus-setting-description]").forEach((node) => {
      const key = node.getAttribute("data-codex-plus-setting-description");
      const item = codexPlusSettingsCatalog.items.find((candidate) => candidate.key === key);
      const description = item ? codexPlusSettingsCatalogDescription(item) : "";
      if (item && node.textContent !== description) node.textContent = description;
    });
    syncCodexPlusNativeFastStatus();
  }

  // 原生 Fast 的自检结果：补丁是否真的打进了当前 Codex。Codex 更新后结构变了时，
  // 用户在这里就能看到「暂未适配」和原因，不用再猜按钮为什么不见了。
  function codexPlusNativeFastStatusText() {
    if (codexPlusBackendSettings.enhancementsEnabled === false || codexPlusBackendSettings.codexAppNativeFastUnlock !== true) return "";
    const state = window.__codexPlusNativeFastPatchState;
    if (window.__codexPlusNativeFastCompiled === "v1" || state === "patched") return "已生效：当前 Codex 版本已适配。";
    if (state === "failed" || state === "unsupported") {
      const reason = String(window.__codexPlusNativeFastFailureReason || window.__codexPlusNativeFastModuleDiagnostics?.reason || "").slice(0, 120);
      return `当前 Codex 版本暂未适配，Fast 开关不会出现${reason ? `（${reason}）` : ""}。请把 codex-plus.log 里 bridge.native_module_patch_prepared 那一行反馈给我们。`;
    }
    return "";
  }

  function syncCodexPlusNativeFastStatus() {
    document.querySelectorAll("[data-codex-plus-native-fast-status]").forEach((node) => {
      const text = codexPlusNativeFastStatusText();
      const failed = String(!!text && !text.startsWith("已生效"));
      if (node.textContent !== text) node.textContent = text;
      if (node.hidden !== !text) node.hidden = !text;
      if (node.dataset.failed !== failed) node.dataset.failed = failed;
    });
  }

  // 与管理器增强页一致：「（实验）」从标题里拿出来做成小标签；说明里写了要重启 Codex 的，标题旁加「需重启」。
  const codexPlusExperimentalTitle = /\s*(?:（实验性?）|\(experimental\))$/i;
  const codexPlusRestartHint = /重启 ?Codex|重新启动 ?Codex|下次启动 ?Codex|restart(?:ing)? Codex|relaunch/i;

  function renderCodexPlusSettingsCatalogTitle(item, description) {
    const experimental = codexPlusExperimentalTitle.test(item.title);
    const tags = [
      experimental ? '<em class="codex-plus-feature-tag" data-tone="experimental">实验</em>' : "",
      item.restart === true || codexPlusRestartHint.test(description) ? '<em class="codex-plus-feature-tag">需重启</em>' : "",
    ].join("");
    return escapeHtml(experimental ? item.title.replace(codexPlusExperimentalTitle, "") : item.title) + tags;
  }

  async function loadCodexPlusUsageStorage(details) {
    const size = details.querySelector("[data-usage-storage-bytes]");
    const path = details.querySelector("[data-usage-storage-directory]");
    details.dataset.loading = "true";
    try {
      const result = await postJson("/settings/usage-history-storage", {});
      if (result.status !== "ok" || !Number.isSafeInteger(result.indexBytes) || result.indexBytes < 0 || typeof result.directory !== "string") {
        throw new Error(result.message || "未取得用量库存储信息");
      }
      size.textContent = result.indexBytes < 1024 ** 2
        ? `${result.indexBytes.toLocaleString()} B`
        : `${(result.indexBytes / 1024 ** 2).toLocaleString(undefined, { maximumFractionDigits: 2 })} MiB`;
      path.textContent = result.directory;
    } catch (error) {
      size.textContent = "读取失败";
      path.textContent = error.message;
    } finally {
      delete details.dataset.loading;
    }
  }

  function renderCodexPlusSettingsCatalogRow(item, options = {}) {
    const title = escapeHtml(item.title);
    const catalogInputs = Array.isArray(item.codexInputs) ? item.codexInputs : [];
    let control = "";
    if (item.control === "bool") {
      const localKey = codexPlusLocalSettingKey(item.key);
      const binding = localKey ? `data-codex-plus-setting="${escapeHtml(localKey)}"` : `data-codex-backend-setting="${escapeHtml(item.key)}"`;
      const toggle = `<button type="button" class="codex-plus-toggle" ${binding} role="switch" aria-checked="false" aria-label="${title}"><span></span></button>`;
      const inputs = catalogInputs.filter((input) => input.id !== "usageHistoryPreciseDays" && input.id !== "usageHistoryStorageMiB")
        .map(renderCodexPlusSettingsCatalogInput).join("");
      control = inputs ? `<div class="codex-plus-width-control">${inputs}${toggle}</div>` : toggle;
    }
    // 不认识的控件类型宁可不显示，也不显示一个点了不生效的行。
    if (!control) return "";
    // 独立模式补丁在增强开启时换一句说明，与管理器一致；切换由 syncCodexPlusSettingsCatalogDescriptions 完成。
    const rawDescription = codexPlusSettingsCatalogDescription(item);
    const description = escapeHtml(rawDescription);
    const descriptionData = item.descriptionWhenEnhancementsOn ? ` data-codex-plus-setting-description="${escapeHtml(item.key)}"` : "";
    const builtin = item.builtin ? ` data-codex-plus-builtin="${escapeHtml(item.builtin)}"` : "";
    const param = options.param ? " data-codex-plus-builtin-param=\"true\"" : "";
    const capacity = catalogInputs.find((input) => input.id === "usageHistoryStorageMiB");
    const storage = item.key === "usageHistoryEnabled" ? `
                <div class="codex-plus-row" data-codex-plus-usage-storage>
                  <div>
                    <div class="codex-plus-row-title">${escapeHtml(capacity?.label || "索引存储")}</div>
                    <div role="status">
                      <div class="codex-plus-row-description">索引占用 <span data-usage-storage-bytes>正在读取</span></div>
                      <div class="codex-plus-row-description">保存目录 <span data-usage-storage-directory></span></div>
                    </div>
                  </div>
                  <div class="codex-plus-width-control codex-plus-usage-policy-control">${renderCodexPlusSettingsCatalogInput(capacity)}</div>
                </div>
                <div class="codex-plus-row">
                  <div>
                    <div class="codex-plus-row-title">逐笔记录保留期</div>
                    <div class="codex-plus-row-description">保留 Token 明细、技能、插件和工具调用信息。到期后按天汇总。延长时，已汇总的明细不会自动补回。</div>
                  </div>
                  <div class="codex-plus-width-control codex-plus-usage-policy-control">${renderCodexPlusSettingsCatalogInput(catalogInputs.find((input) => input.id === "usageHistoryPreciseDays"))}</div>
                </div>
                <div class="codex-plus-row">
                  <div>
                    <div class="codex-plus-row-title">工具输入和输出</div>
                    <div class="codex-plus-row-description">只保留最近 24 小时的工具输入输出索引，详情从原会话读取。空间不足时优先回收较早的工具日汇总。</div>
                  </div>
                </div>` : "";
    return `
                <div class="codex-plus-row" data-codex-plus-setting-row="${escapeHtml(item.key)}"${builtin}${param}>
                  <div><div class="codex-plus-row-title">${renderCodexPlusSettingsCatalogTitle(item, rawDescription)}</div><div class="codex-plus-row-description"${descriptionData}>${description}</div>${item.key === "codexAppNativeFastUnlock" ? '<div class="codex-plus-row-description" data-codex-plus-native-fast-status="true" role="status" hidden></div>' : ""}</div>
                  ${control}
                </div>${storage}`;
  }

  /**
   * 清单顶层的内置拓展（桌宠、听写、梦皮……）：设置字段是本页认识的顶层字段就照常绑定设置；
   * 嵌套字段或没有设置字段的，开关走 /user-scripts/set-script-enabled（后台按清单写设置或记在用户脚本配置里）。
   */
  function renderCodexPlusBuiltinEntryRow(entry) {
    const title = escapeHtml(entry.title);
    const setting = typeof entry.setting === "string" && !entry.setting.includes(".") ? entry.setting : "";
    const localKey = setting ? codexPlusLocalSettingKey(setting) : "";
    const binding = localKey ? `data-codex-plus-setting="${escapeHtml(localKey)}"`
      : setting ? `data-codex-backend-setting="${escapeHtml(setting)}"`
        : `data-codex-builtin-extension="${escapeHtml(entry.id)}"`;
    return `
                <div class="codex-plus-row" data-codex-plus-builtin="${escapeHtml(entry.id)}">
                  <div><div class="codex-plus-row-title">${renderCodexPlusSettingsCatalogTitle(entry, entry.description || "")}</div><div class="codex-plus-row-description">${escapeHtml(entry.description || "")}</div></div>
                  <button type="button" class="codex-plus-toggle" ${binding} role="switch" aria-checked="false" aria-label="${title}"><span></span></button>
                </div>`;
  }

  /** 一组里的行：不属于任何内置拓展的设置（总开关等）在前，然后每个内置拓展一行、子选项紧跟其后。 */
  function renderCodexPlusSettingsGroupRows(catalog, group) {
    const visible = (item) => item.group === group.id && codexPlusSettingsCatalogItemVisible(item);
    const params = (id) => catalog.items.filter((item) => item.parent === id && codexPlusSettingsCatalogItemVisible(item))
      .map((item) => renderCodexPlusSettingsCatalogRow(item, { param: true }));
    return [
      ...catalog.items.filter((item) => visible(item) && !item.builtin && !item.parent).map((item) => renderCodexPlusSettingsCatalogRow(item)),
      ...catalog.items.filter((item) => visible(item) && item.builtin)
        .flatMap((item) => [renderCodexPlusSettingsCatalogRow(item), ...params(item.builtin)]),
      ...(catalog.builtins || []).filter((entry) => entry.group === group.id)
        .flatMap((entry) => [renderCodexPlusBuiltinEntryRow(entry), ...params(entry.id)]),
    ].filter(Boolean);
  }

  /**
   * 内置拓展开关（走用户脚本配置的那几个）与「已被顶替」提示，按后台给的内置拓展现状同步。
   * 提示写在说明元素的 data-codex-builtin-replaced 上，由样式接在说明后面显示。
   */
  function syncCodexPlusBuiltinExtensionToggles() {
    const builtins = new Map((codexPlusUserScripts.builtins || []).map((builtin) => [builtin.id, builtin]));
    document.querySelectorAll(".codex-plus-toggle[data-codex-builtin-extension]").forEach((button) => {
      const builtin = builtins.get(button.getAttribute("data-codex-builtin-extension"));
      const enabled = String(builtin ? !!builtin.enabled : true);
      button.dataset.pending = String(!builtin);
      if (button.dataset.enabled === enabled) return;
      button.dataset.enabled = enabled;
      button.setAttribute("aria-checked", enabled);
    });
    document.querySelectorAll(".codex-plus-row[data-codex-plus-builtin]").forEach((row) => {
      const replacedBy = builtins.get(row.getAttribute("data-codex-plus-builtin"))?.replacedBy;
      const text = replacedBy ? `已被 ${String(replacedBy).replace(/^user:/, "")} 顶替` : "";
      const description = row.querySelector(".codex-plus-row-description");
      if (!description) return;
      if (text) description.setAttribute("data-codex-builtin-replaced", text);
      else description.removeAttribute("data-codex-builtin-replaced");
    });
  }

  /** 页面里切换一个内置拓展：后台写设置或用户脚本配置，回来后同步页面里的停用状态与设置。 */
  async function setCodexPlusBuiltinExtensionEnabled(id, enabled) {
    await loadUserScripts("/user-scripts/set-script-enabled", { key: `builtin:${id}`, enabled });
    const builtins = codexPlusUserScripts.builtins || [];
    if (builtins.length) {
      window.__codexPlusBuiltinExtensionState = {
        disabled: builtins.filter((builtin) => !builtin.setting && !builtin.enabled).map((builtin) => builtin.id),
        replacedBy: Object.fromEntries(builtins.filter((builtin) => builtin.replacedBy).map((builtin) => [builtin.id, builtin.replacedBy])),
      };
      try { window.__codexPlusSyncBuiltinExtensions?.(); } catch {}
    }
    // 有设置字段的（桌宠、听写）后台改的是设置，重新读一次让功能跟上。
    await loadBackendSettings();
    syncCodexPlusBuiltinExtensionToggles();
  }

  /** 设置页主体：清单就绪时按分组生成，否则显示读取中 / 读取失败与重试。 */
  function renderCodexPlusSettingsCatalogSections() {
    const catalog = codexPlusSettingsCatalog;
    if (!catalog) {
      const failed = codexPlusSettingsCatalogStatus.status === "failed";
      const message = codexPlusSettingsCatalogStatus.message;
      const state = failed
        ? `
                <div class="codex-plus-row" data-codex-plus-settings-catalog-state="failed" role="alert">
                  <div><div class="codex-plus-row-title">设置项读取失败</div><div class="codex-plus-row-description">无法从 Codex++ 后端读取设置项清单${message ? `：${escapeHtml(message)}` : ""}。确认后端已连接后重试。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-plus-settings-catalog-retry="true">重试</button>
                </div>`
        : `
                <div class="codex-plus-row" data-codex-plus-settings-catalog-state="loading" role="status">
                  <div><div class="codex-plus-row-title">正在读取设置项…</div><div class="codex-plus-row-description">设置项清单由 Codex++ 后端提供，读取完成后显示。</div></div>
                </div>`;
      return renderCodexPlusSettingsGroup("core", "常规", renderCodexPlusSettingsSection("设置", renderCodexPlusBackendStatusRow() + state),
        ' data-codex-plus-settings-catalog="group"');
    }
    // #686：「上下文感知建议」读写的是 Codex 自己的设置，不在后端清单里；放进 stepwise 组，
    // 清单没有这一组时退到 chat 组，再不行放第一组，保证总能看到。
    const ambientGroupId = ["stepwise", "chat"].find((id) => catalog.groups.some((group) => group.id === id))
      || catalog.groups[0]?.id;
    // typeof 守卫：测试会把这段单独切出来执行，那时用户脚本状态不在作用域里。
    if (typeof codexPlusUserScriptsLoaded !== "undefined" && !codexPlusUserScriptsLoaded) void loadUserScripts();
    return catalog.groups.map((group, index) => {
      const rows = renderCodexPlusSettingsGroupRows(catalog, group);
      if (group.id === ambientGroupId) rows.push(renderCodexAmbientSuggestionsRow());
      if (index === 0) rows.unshift(renderCodexPlusBackendStatusRow());
      const section = rows.length ? renderCodexPlusSettingsSection(group.title, rows.join(""), group.description || "") : "";
      // 听写与 stepwise 的服务配置（46-service-settings.js）紧跟在各自分组后面，和分组同一页。
      const service = typeof renderCodexPlusServiceSettingsAfterGroup === "function" ? renderCodexPlusServiceSettingsAfterGroup(group.id) : "";
      return section || service
        ? renderCodexPlusSettingsGroup(group.id, group.title, section + service, ' data-codex-plus-settings-catalog="group"')
        : "";
    }).join("");
  }

  /** 清单状态变化后重建设置页里由清单生成的部分，并把开关状态同步回来。 */
  function refreshCodexPlusSettingsCatalogView() {
    let rendered = false;
    document.querySelectorAll('[data-codex-plus-panel="home"]').forEach((panel) => {
      // 顺序：清单分组 → 拓展分区 → 拓展设置 → 工具与关于。清单分组插在后三者里最靠前的那个前面。
      const anchor = panel.querySelector('[data-codex-plus-settings-extension], [data-codex-plus-settings-group="ext-menu"], [data-codex-plus-settings-tools]');
      if (!anchor) return;
      const retryFocused = !!document.activeElement?.closest?.("[data-codex-plus-settings-catalog-retry]");
      panel.querySelectorAll("[data-codex-plus-settings-catalog]").forEach((node) => node.remove());
      anchor.insertAdjacentHTML("beforebegin", renderCodexPlusSettingsCatalogSections());
      if (retryFocused) {
        panel.querySelector("[data-codex-plus-settings-catalog-retry], [data-codex-plus-settings-catalog] .codex-plus-toggle")?.focus({ preventScroll: true });
      }
      rendered = true;
    });
    if (!rendered) return;
    // 清单读完后分组才齐：左栏跟着重画，记住的分组也在这时才能落位。
    refreshCodexPlusSettingsNav();
    syncCodexPlusSettingsGroups();
    renderBackendStatus();
    refreshCodexPlusBackendToggles();
    syncCodexAmbientSuggestionsToggles();
    syncCodexPlusBuiltinExtensionToggles();
  }

  function openCodexPlusModal(options = {}) {
    const pageMode = options.page === true;
    const initialTab = codexPlusModalTab(options.tab);
    closeCodexPlusPage();
    document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
    document.querySelectorAll(`.${codexPlusPageClass}, [data-codex-plus-dialog="true"]`).forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = pageMode ? codexPlusPageClass : "codex-plus-modal-overlay";
    overlay.dataset.codexPlusPage = String(pageMode);
    applyCodexPlusTheme(overlay);
    // 跟随 Codex 的界面缩放。必须在写 innerHTML 之前设好，否则内部那些
    // calc(100% / var(--codex-plus-zoom-inverse)) 会先按 1 算一遍再被 zoom 放大。
    applyCodexPlusZoom(overlay);
    // 下拉箭头与勾选图标使用已安装的 Lucide ChevronDown / Check（ISC）路径。
    overlay.innerHTML = `
      <div class="codex-plus-modal-content" role="dialog" aria-modal="true" aria-label="Codex++">
        <div class="codex-plus-modal-header">
          <div class="codex-plus-modal-title"><span class="codex-plus-backend-indicator" data-codex-backend-indicator="true" data-status="checking"></span><span data-codex-plus-version="true">Codex++ ${codexPlusVersion}</span></div>
          ${pageMode ? "" : `<button type="button" class="codex-plus-modal-close" aria-label="关闭">×</button>`}
        </div>
        <div class="codex-plus-modal-body">
          <div class="codex-plus-panel codex-plus-settings" data-codex-plus-panel="home" data-codex-plus-settings-sections="${escapeHtml(codexPlusSettingsSectionsFingerprint())}">
            ${renderCodexPlusSettingsCatalogSections()}
            ${renderCodexPlusSettingsSectionGroups()}
            <div class="codex-plus-settings-group" data-codex-plus-settings-group="ext-menu" data-codex-plus-settings-label="拓展设置" data-active="${String(codexPlusSettingsActiveGroup === "ext-menu")}">
            <section class="codex-plus-settings-section" aria-label="拓展设置" data-codex-plus-ext-menu-host="true">
              <h2 class="codex-plus-settings-section-title">拓展设置</h2>
              <p class="codex-plus-settings-section-description">已启用的拓展注册的开关与操作。</p>
            ${renderCodexPlusExtensionMenuRows()}
            </section>
            </div>
            <div class="codex-plus-settings-group" data-codex-plus-settings-group="tools" data-codex-plus-settings-label="工具与关于" data-active="${String(codexPlusSettingsActiveGroup === "tools")}" data-codex-plus-settings-tools="true">
            <section class="codex-plus-settings-section" aria-label="工具与关于">
              <h2 class="codex-plus-settings-section-title">工具与关于</h2>
              <div class="codex-plus-settings-card">
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">管理工具</div><div class="codex-plus-row-description">增强总开关、供应商、本地服务与会话维护在管理工具里设置。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-open-manager="true">打开管理工具</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">刷新 UI 配置</div><div class="codex-plus-row-description">手动请求当前 Statsig 客户端重新评估远程配置，并显示 enable_i18n 的结果。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-refresh-statsig="true">刷新 UI 配置</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">打开 DevTools</div><div class="codex-plus-row-description">打开当前 Codex 页面开发者工具，方便查看用户拓展报错。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-open-devtools="true">打开 DevTools</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">关于 Codex++</div><div class="codex-plus-about">Codex++ 是通过外部 launcher 注入的增强菜单，不修改 Codex App 原始安装文件。<br>Build: <span data-codex-plus-build="true">${codexPlusBuild}</span><br>GitHub: <a href="https://github.com/BigPizzaV3/CodexPlusPlus" target="_blank" rel="noreferrer">https://github.com/BigPizzaV3/CodexPlusPlus</a><br>Discord: <a href="https://discord.gg/y96kX7A76v" target="_blank" rel="noreferrer">https://discord.gg/y96kX7A76v</a><br>Telegram: <a href="https://t.me/CodexPlusPlus" target="_blank" rel="noreferrer">https://t.me/CodexPlusPlus</a></div></div>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Discord 社区</div><div class="codex-plus-row-description">加入 Discord 获取更新消息、反馈问题或交流使用体验。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-plus-discord="true">打开 Discord</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Telegram 频道</div><div class="codex-plus-row-description">加入 Telegram 获取更新消息和交流使用体验。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-plus-telegram="true">打开 Telegram</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">提出问题</div><div class="codex-plus-row-description">打开 GitHub Issues 反馈问题或建议。</div></div>
                  <button type="button" class="codex-plus-issue-button" data-codex-plus-issue="true">提出问题</button>
                </div>
              </div>
            </section>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="${codexPlusExtensionsTab}" hidden>
            <div class="codex-plus-discovery codex-plus-extensions-page">
              <div class="codex-plus-discovery-header">
                <div><h1 class="codex-plus-discovery-title">拓展</h1><p class="codex-plus-discovery-description"><strong>万物皆拓展</strong>：界面、命令、模型工具、钩子、网关与后台都能由拓展接管，在对话里说一句就能造一个。</p></div>
                <div class="codex-plus-discovery-toolbar">
                  <label class="codex-plus-discovery-search">${codexPlusDiscoveryIcons.search}<input type="search" data-codex-extensions-search="true" aria-label="搜索拓展" placeholder="搜索拓展" value="${escapeHtml(codexPlusExtensionsQuery)}" spellcheck="false" /></label>
                  <button type="button" class="codex-plus-discovery-icon-button" data-codex-market-refresh="true" title="刷新拓展" aria-label="刷新拓展">${codexPlusDiscoveryIcons.refresh}</button>
                </div>
              </div>
              <div class="codex-plus-extensions-detail" data-codex-plus-extensions-detail="true">${renderCodexPlusExtensionsDetail()}</div>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="sponsor" hidden>
            <div class="codex-plus-discovery codex-plus-sponsor-page">
              <div class="codex-plus-discovery-header">
                <div><h1 class="codex-plus-discovery-title">推荐内容</h1><p class="codex-plus-discovery-description codex-plus-sponsor-text">以下推荐来自支持 Codex++ 继续维护的合作方。</p></div>
              </div>
              <div class="codex-plus-ad-remote">${renderCodexPlusAds()}</div>
            </div>
          </div>
        </div>
      </div>
    `;
    if (typeof mountCodexPlusSettingsPanel === "function") mountCodexPlusSettingsPanel(overlay.querySelector("[data-codex-plus-extensions-detail]"));
    overlay.querySelectorAll(".codex-plus-toggle[data-codex-plus-setting], .codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      button.setAttribute("role", "switch");
      button.setAttribute("aria-checked", "false");
      button.setAttribute("aria-label", button.closest(".codex-plus-row")?.querySelector(".codex-plus-row-title")?.textContent || "设置");
    });
    const closeButton = overlay.querySelector(".codex-plus-modal-close");
    closeButton?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      overlay.remove();
      if (pageMode) setCodexPlusSidebarNavActive(false);
    }, true);
    overlay.addEventListener("input", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (typeof handleCodexPlusExtensionSourcesInput === "function" && handleCodexPlusExtensionSourcesInput(target)) return;
      const searchInput = target?.closest("[data-codex-extensions-search]");
      if (searchInput) {
        codexPlusExtensionsQuery = searchInput.value;
        codexPlusExtensionsSelected = null;
        refreshCodexPlusExtensionsView();
        return;
      }
      const widthInput = target?.closest("[data-codex-plus-conversation-view-width]");
      if (widthInput) setConversationViewWidth(widthInput.value);
    }, true);
    overlay.addEventListener("change", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const usageUnit = target?.closest("[data-codex-plus-usage-unit]");
      if (usageUnit) {
        if (usageUnit.disabled) return;
        const input = usageUnit.previousElementSibling;
        const scale = Number(usageUnit.value);
        codexPlusUsageStorageScale = scale;
        input.min = input.step = String(1 / scale);
        input.max = String(4294967295 / scale);
        input.value = String((codexPlusBackendSettings.usageHistoryStorageMiB ?? 2048) / scale);
        return;
      }
      const policy = target?.closest("[data-codex-plus-usage-policy]");
      if (policy) {
        if (policy.disabled) return;
        const key = policy.dataset.codexPlusUsagePolicy;
        const unit = key === "usageHistoryStorageMiB" ? policy.nextElementSibling : null;
        const scale = unit ? Number(unit.value) : 1;
        const value = policy.valueAsNumber * scale;
        const previous = codexPlusBackendSettings[key] ?? (unit ? 2048 : Number(policy.defaultValue));
        if (!Number.isInteger(value) || !policy.validity.valid) {
          policy.reportValidity();
          policy.value = String(previous / scale);
          return;
        }
        if (value === previous) return;
        policy.dataset.saving = "true";
        policy.disabled = true;
        if (unit) unit.disabled = true;
        void postJson("/settings/set", { [key]: value }).then((saved) => {
          if (saved?.[key] !== value) throw new Error(saved?.message || "设置未保存");
          codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
          emitCodexPlusSettingsChanged(key, value, true);
        }).catch((error) => {
          policy.value = String((codexPlusBackendSettings[key] ?? previous) / scale);
          showToast(`保存用量设置失败：${error.message}`, { type: "error" });
        }).finally(() => {
          delete policy.dataset.saving;
          policy.disabled = codexPlusBackendSettings.usageHistoryEnabled !== true;
          if (unit) unit.disabled = policy.disabled;
        });
        return;
      }
      if (typeof handleCodexPlusExtensionConfigChange === "function" && handleCodexPlusExtensionConfigChange(target)) return;
      if (typeof handleCodexPlusServiceSettingsChange === "function" && handleCodexPlusServiceSettingsChange(target)) return;
      const widthInput = target?.closest("[data-codex-plus-conversation-view-width]");
      if (widthInput) {
        const width = normalizeConversationViewWidth(widthInput.value);
        widthInput.value = String(width || conversationViewWidth());
        setConversationViewWidth(widthInput.value);
      }
      // 次数/间隔输入只在 change（失焦/回车）时提交：input 事件逐字符提交会把
      // 用户输入 "15" 的过程保存成 1。
      const attemptsInput = target?.closest("[data-codex-plus-capacity-retry-max-attempts]");
      if (attemptsInput) {
        attemptsInput.value = String(setCapacityAutoRetryMaxAttempts(attemptsInput.value));
      }
      const intervalInput = target?.closest("[data-codex-plus-capacity-retry-interval]");
      if (intervalInput) {
        intervalInput.value = String(setCapacityAutoRetryIntervalSeconds(intervalInput.value));
      }
    }, true);
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      // 拓展注册的菜单项。放在最前面是因为它的判定完全基于自己的 data 属性，
      // 与下面那些内置分支不会重叠；万一将来重叠，也应当由拓展优先拿到。
      if (handleCodexPlusExtensionMenuClick(target)) return;
      // 拓展详情的页签、设置表单开关、README 页内锚点。
      if (typeof handleCodexPlusExtensionDetailClick === "function" && handleCodexPlusExtensionDetailClick(target, event)) return;
      // 社区 / 我发布的 / dsh 插件分栏的按钮。
      if (typeof handleCodexPlusExtensionSourcesClick === "function" && handleCodexPlusExtensionSourcesClick(target, event)) return;
      const extensionsBrowse = target?.closest("[data-codex-extensions-browse]");
      if (extensionsBrowse) {
        codexPlusExtensionsFilter = codexPlusExtensionsFilterValue(extensionsBrowse.dataset.codexExtensionsBrowse);
        codexPlusExtensionsSelected = null;
        refreshCodexPlusExtensionsView();
        if (typeof codexPlusExtensionSourceEnsureLoaded === "function") codexPlusExtensionSourceEnsureLoaded(codexPlusExtensionsFilter);
        return;
      }
      // 设置页左栏「页面」一节：打开收进 Codex++ 入口的那一页。
      const pageLink = target?.closest("[data-codex-plus-page-link]");
      if (pageLink) {
        openCodexPlusFoldedPage(pageLink.getAttribute("data-codex-plus-page-link"));
        return;
      }
      // 设置页左栏：切换右边显示的分组。
      const settingsNav = target?.closest("[data-codex-plus-settings-nav]");
      if (settingsNav) {
        selectCodexPlusSettingsGroup(settingsNav.getAttribute("data-codex-plus-settings-nav"));
        return;
      }
      // 左面板的分组导航（拓展页）。
      const pageNav = target?.closest("[data-codex-plus-page-nav]");
      if (pageNav) {
        selectCodexPlusTab(pageNav.getAttribute("data-codex-plus-page-nav"));
        return;
      }
      if (target?.closest("[data-codex-plus-settings-catalog-retry]")) {
        void loadCodexPlusSettingsCatalog();
        return;
      }
      if (typeof handleCodexPlusServiceSettingsClick === "function" && handleCodexPlusServiceSettingsClick(target)) return;
      if (target?.closest("[data-codex-open-devtools]")) {
        postJson("/devtools/open", {});
        return;
      }
      if (target?.closest("[data-codex-open-manager]")) {
        openManagerFromCodex();
        return;
      }
      const refreshStatsigButton = target?.closest("[data-codex-refresh-statsig]");
      if (refreshStatsigButton) {
        const button = refreshStatsigButton;
        if (button.dataset.pending === "true") return;
        button.dataset.pending = "true";
        const originalLabel = button.textContent;
        button.textContent = "刷新中…";
        Promise.resolve(window.__codexPlusRefreshStatsigUiConfig?.())
          .then((result) => {
            if (!result || result.status === "unsupported") {
              showToast("当前 Codex 构建没有可用的 Statsig 刷新接口", null);
            } else if (result.status === "ok" && result.enable_i18n === true) {
              showToast("UI 配置已刷新，enable_i18n=true", null);
              // 只有用户主动点击且远端明确返回 true 时刷新页面，让 Codex
              // 重新走语言包加载 gate；不在启动/网络恢复路径自动触发。
              window.setTimeout(() => window.location.reload(), 150);
            } else if (result.status === "ok" && result.enable_i18n === false) {
              showToast("UI 配置已刷新，enable_i18n=false", null);
            } else {
              // showToast 的第二个参数若是字符串会被当成撤销 token（旧签名），
              // 错误详情必须拼进正文并以对象参数标明类型。
              const detail = result.errors?.join("；");
              showToast(`UI 配置刷新${result.status === "partial" ? "部分完成" : "失败"}${detail ? `：${detail}` : ""}`, { type: result.status === "partial" ? "warn" : "error" });
            }
          })
          .catch((error) => showToast(`UI 配置刷新失败：${error?.message || String(error)}`, { type: "error" }))
          .finally(() => {
            button.dataset.pending = "false";
            button.textContent = originalLabel || "刷新 UI 配置";
          });
        return;
      }
      // 推荐卡片用 window.open 而非原生 <a target="_blank">：Codex 是 Electron
      // 应用，原生新窗口跳转在它的 webview 里不会交给系统浏览器（同页的
      // Discord / Telegram / Issues 按钮也一律走 window.open）。
      const adCard = target?.closest("[data-codex-plus-ad-url]");
      if (adCard) {
        // 无论 URL 合不合法都先掐掉 <a> 的默认行为：留下它，非 http(s) 的
        // scheme 就会走原生跳转，而不是被下面这层挡掉。
        event.preventDefault();
        const adUrl = adCard.getAttribute("data-codex-plus-ad-url") || "";
        if (isCodexPlusAdWebUrl(adUrl)) {
          window.open(adUrl, "_blank", "noopener,noreferrer");
        }
        return;
      }
      if (target?.closest("[data-codex-plus-discord]")) {
        window.open("https://discord.gg/y96kX7A76v", "_blank");
        return;
      }
      if (target?.closest("[data-codex-plus-telegram]")) {
        window.open("https://t.me/CodexPlusPlus", "_blank");
        return;
      }
      const issueButton = target?.closest("[data-codex-plus-issue]");
      if (issueButton) {
        const issueUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/issues";
        window.open(issueUrl, "_blank");
        return;
      }
      const builtinToggle = target?.closest("[data-codex-builtin-extension]");
      if (builtinToggle) {
        if (builtinToggle.dataset.pending === "true") return;
        void setCodexPlusBuiltinExtensionEnabled(builtinToggle.getAttribute("data-codex-builtin-extension"), builtinToggle.dataset.enabled !== "true");
        return;
      }
      const userScriptToggle = target?.closest("[data-codex-user-script-key]");
      if (userScriptToggle) {
        if (userScriptToggle.dataset.pending === "true") return;
        userScriptToggle.dataset.pending = "true";
        void setUserScriptEnabled(userScriptToggle.getAttribute("data-codex-user-script-key"), userScriptToggle.dataset.enabled !== "true")
          .finally(() => { userScriptToggle.dataset.pending = "false"; });
        return;
      }
      // 市场条目的「安装」。id 挂在行/按钮上，点按钮才算。
      const marketInstall = target?.closest("[data-codex-market-install]");
      if (marketInstall) {
        void installScriptFromMarket(marketInstall.getAttribute("data-codex-market-install"));
        return;
      }
      // 精选 / 官方拓展包的「安装」：先准备再弹确认框。
      const packageInstall = target?.closest("[data-codex-package-install]");
      if (packageInstall) {
        if (!packageInstall.disabled) void installPackageFromMarket(packageInstall.getAttribute("data-codex-package-install"));
        return;
      }
      const extensionsRefresh = target?.closest("[data-codex-market-refresh]");
      if (extensionsRefresh) {
        // 顺带丢掉 README 缓存：改过的 README 重新打开详情就能看到。
        if (typeof clearCodexPlusExtensionReadmes === "function") clearCodexPlusExtensionReadmes();
        void loadScriptMarket(true);
        if (typeof codexPlusExtensionSourceEnsureLoaded === "function") codexPlusExtensionSourceEnsureLoaded(codexPlusExtensionsFilter, true);
        return;
      }
      const extensionsUninstall = target?.closest("[data-codex-extensions-uninstall]");
      if (extensionsUninstall) {
        void uninstallUserScript(extensionsUninstall.getAttribute("data-codex-extensions-uninstall"));
        return;
      }
      // 左面板点行 = 选中并在右侧显示详情。放在安装/卸载之后，
      // 免得点了行内的按钮又被当成一次选中。
      const extensionsSelect = target?.closest("[data-codex-extensions-select]");
      if (extensionsSelect) {
        const [kind, ...rest] = extensionsSelect.getAttribute("data-codex-extensions-select").split(":");
        codexPlusExtensionsSelected = { kind, key: rest.join(":") };
        if (typeof resetCodexPlusExtensionsDetailTab === "function") resetCodexPlusExtensionsDetailTab();
        refreshCodexPlusExtensionsView();
        overlay.querySelector(".codex-plus-extensions-back")?.focus({ preventScroll: true });
        return;
      }
      const ambientToggle = target?.closest("[data-codex-app-ambient-suggestions]");
      if (ambientToggle) {
        if (ambientToggle.disabled || ambientToggle.dataset.pending === "true") return;
        void setCodexAmbientSuggestionsEnabled(ambientToggle.dataset.enabled !== "true");
        return;
      }
      const toggle = target?.closest("[data-codex-plus-setting]");
      if (toggle) {
        if (toggle.disabled || toggle.dataset.pending === "true") return;
        const key = toggle.getAttribute("data-codex-plus-setting");
        setCodexPlusSetting(key, !codexPlusSettings({ ignoreBuiltins: true })[key]);
        return;
      }
      const backendToggle = target?.closest("[data-codex-backend-setting]");
      if (backendToggle) {
        const key = backendToggle.getAttribute("data-codex-backend-setting");
        const next = !codexPlusBackendSettings[key];
        // 原生浏览器请求标识会把会话标识发给目标网站，开启前要用户明确同意（以前在管理器里确认）。
        if (next && key === "codexAppNativeBrowserRequireIdentification") {
          void Promise.resolve(codexPlusAsk({
            title: "启用原生 Edge / Chrome 请求标识兼容？",
            message: "受控标签页的目标网站可收到包含会话标识的 x-browser-agent 请求头。原生扩展会保留开启状态；关闭此兼容选项或恢复服务文件不会关闭扩展已保存的标识设置。\n\n此选项不代替站点授权、企业策略或浏览器操作审批。仅适配指定 Windows 原生运行时的 Edge / Chrome 稳定版扩展。\n\n保存后需由你关闭并重新启动 Codex / Codex++ 才能完成验收，不会自动重启。",
            options: [{ value: "enable", label: "启用" }],
          })).then((answer) => {
            if (answer === "enable") setBackendSetting(key, true);
          });
          return;
        }
        setBackendSetting(key, next);
        return;
      }
    }, true);
    // 图标加载失败的回退：error 不冒泡，只能捕获阶段委托。
    overlay.addEventListener("error", handleExtensionIconError, true);
    document.body.appendChild(overlay);
    if (pageMode) {
      observeCodexPlusPageLayout(overlay);
      // 必须在 selectCodexPlusTab 之前建好两栏，否则刷新左面板时找不到容器。
      installCodexPlusPageLayout(overlay, initialTab);
    }
    if (!codexPlusAdsLoaded) fetchCodexPlusAds();
    selectCodexPlusTab(initialTab);
    // 必须在 selectCodexPlusTab 之后：激活态要靠 data-codex-plus-active-tab
    // 判断当前是 Codex++ 还是「拓展」，提前调用会永远落到 home 上。
    if (pageMode) setCodexPlusSidebarNavActive(true, codexPlusActiveEntry() || "home");
    renderCodexPlusMenu();
    refreshCodexPlusBackendToggles();
    renderBackendStatus();
    loadUserScripts();
    // 每次打开都重读一次：用户可能在 Codex 设置页（官方登录时可见）改过。
    void loadCodexAmbientSuggestionsSetting();
    // 清单已缓存时上面的模板已经按清单渲染；否则读完后再替换“正在读取”占位。
    if (!codexPlusSettingsCatalog) void loadCodexPlusSettingsCatalog();
  }

  function openCodexPlusPage() {
    openCodexPlusModal({ page: true });
  }

  /** 「拓展」页面：从弹窗里拆出来的用户脚本，形态对齐 VSCode 的扩展面板。 */
  function openCodexPlusExtensions() {
    codexPlusExtensionsSelected = null;
    openCodexPlusModal({ page: true, tab: codexPlusExtensionsTab });
  }

  /** 「推荐内容」页面：从弹窗的二级 tab 提出来，成为图标栏上的一级入口。 */
  function openCodexPlusSponsor() {
    openCodexPlusModal({ page: true, tab: codexPlusSponsorTab });
  }

  function closeCodexPlusPage() {
    clearTimeout(window.__codexPlusPageNavigationCloseTimer);
    window.__codexPlusPageNavigationCloseTimer = null;
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = null;
    window.__codexPlusPageLayoutObserver?.disconnect();
    window.__codexPlusPageLayoutObserver = null;
    window.__codexPlusPageLayoutTargets = null;
    // typeof 守卫：测试会把关页函数单独切出来执行。
    if (typeof unmountCodexPlusSettingsSections === "function") unmountCodexPlusSettingsSections();
    document.querySelectorAll(`.${codexPlusPageClass}`).forEach((node) => node.remove());
    if (typeof unmountCodexPlusSettingsPanel === "function") unmountCodexPlusSettingsPanel();
    setCodexPlusSidebarNavActive(false);
    restoreCodexPlusPageBackButton();
  }

  function codexPlusNativeBackButton() {
    const labels = new Set(["back", "go back", "返回", "后退"]);
    const selector = selectors.appHeader.split(",").map((header) => `${header.trim()} button`).join(", ");
    const buttons = Array.from(document.querySelectorAll(selector))
      .filter((button) => !button.closest(`.${codexPlusPageClass}, [data-codex-plus-ext]`));
    // 宿主的前进按钮复用返回箭头并水平翻转；路径识别不依赖界面语言。
    const backIcon = 'svg:not([class~="-scale-x-100"]) path[d^="M6.96231 2.96214C7.16727 2.7574"]';
    return buttons.find((button) => button.querySelector(backIcon)) || buttons.find((button) =>
      labels.has(String(button.getAttribute("aria-label") || button.title || "").trim().toLowerCase())) || null;
  }

  function restoreCodexPlusPageBackButton() {
    const state = window.__codexPlusPageBackButtonState;
    window.__codexPlusPageBackButtonState = null;
    if (!state) return;
    for (const [name, value] of state.attributes) {
      if (value === null) state.button.removeAttribute(name);
      else state.button.setAttribute(name, value);
    }
  }

  function syncCodexPlusPageBackButton() {
    if (window.__codexPlusPageNavigationCloseTimer != null) return;
    if (!document.querySelector(`.${codexPlusPageClass}`)) {
      restoreCodexPlusPageBackButton();
      return;
    }
    const button = codexPlusNativeBackButton();
    if (!button) return;
    if (window.__codexPlusPageBackButtonState?.button !== button) {
      restoreCodexPlusPageBackButton();
      window.__codexPlusPageBackButtonState = {
        button,
        attributes: ["disabled", "aria-disabled", "tabindex", "data-disabled"]
          .map((name) => [name, button.getAttribute(name)]),
      };
    }
    // 覆盖页自身可返回，即使原生历史尚无上一页也必须能点击。
    button.removeAttribute("disabled");
    button.removeAttribute("aria-disabled");
    button.removeAttribute("data-disabled");
    if (button.getAttribute("tabindex") === "-1") button.removeAttribute("tabindex");
  }

  function closeCodexPlusPageAfterNativeNavigation() {
    const page = document.querySelector(`.${codexPlusPageClass}`);
    if (!page) return;
    clearTimeout(window.__codexPlusPageNavigationCloseTimer);
    // 原生导航前撤销临时属性，避免延迟关闭覆盖导航提交后的返回状态。
    restoreCodexPlusPageBackButton();
    const timer = setTimeout(() => {
      if (window.__codexPlusPageNavigationCloseTimer !== timer) return;
      window.__codexPlusPageNavigationCloseTimer = null;
      if (document.querySelector(`.${codexPlusPageClass}`) === page) closeCodexPlusPage();
    }, 0);
    window.__codexPlusPageNavigationCloseTimer = timer;
  }

  function installCodexPlusPageNavigationCloseHandler() {
    syncCodexPlusPageBackButton();
    document.removeEventListener("click", window.__codexPlusPageNavigationCloseHandler, true);
    window.removeEventListener("click", window.__codexPlusPageNavigationCloseHandler, true);
    window.__codexPlusPageNavigationCloseHandler = (event) => {
      if (!document.querySelector(`.${codexPlusPageClass}`)) return;
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const backButton = codexPlusNativeBackButton();
      if (backButton && target?.closest("button") === backButton) {
        // 原生界面仍在覆盖页下方，关闭即可；继续后退会跳过进入前的界面。
        event.preventDefault();
        event.stopImmediatePropagation();
        closeCodexPlusPage();
        return;
      }
      // 侧栏收起时原生侧栏能悬浮在页面之上（10-style.js），点里面的入口同样是去原生页面。
      const nativeSidebarItem = target?.closest("[data-sidebar-navigation-root] :is(button, a)");
      if (!target?.closest(selectors.sidebarThread) && !(nativeSidebarItem && !nativeSidebarItem.closest(`#${codexPlusSidebarNavId}`))) return;
      // Let Codex's own click handler update its route before removing our page.
      closeCodexPlusPageAfterNativeNavigation();
    };
    window.addEventListener("click", window.__codexPlusPageNavigationCloseHandler, true);
  }

  function installCodexPlusSidebarNavigation() {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => node.remove());
    // 旧版的侧边栏会话列表在 aside 里带 role="navigation"。新版把这个 role 挪去了
    // 缩略图面板/演示目录，所以留一条限定在 aside 内的兜底。
    // 注意：新版图标栏也是 aside 里的 <nav>，且文档顺序在前，而 querySelector 的选择器
    // 列表是按文档顺序取首个命中项的——必须显式排除图标栏，否则会挂到它上面。
    const navigation = document.querySelector('aside.app-shell-left-panel nav[role="navigation"]')
      || Array.from(document.querySelectorAll("aside.app-shell-left-panel nav"))
        .find((nav) => !nav.hasAttribute("data-app-navigation-rail"))
      || null;
    if (!navigation) return;
    const navButtons = Array.from(navigation.querySelectorAll("button"));
    const pluginButton = navButtons.find((button) => {
      if (button.querySelector(selectors.pluginSvgPath)) return true;
      const label = (button.getAttribute("aria-label") || button.textContent || "").trim();
      return /^(插件|Plugins)$/i.test(label);
    });
    const insertionButton = pluginButton || navButtons.find((button) => {
      const label = (button.getAttribute("aria-label") || button.textContent || "").replace(/\s+/g, " ").trim();
      return /^(已安排|Scheduled|拉取请求|Pull requests|新对话|New chat)$/i.test(label);
    });
    if (navigation.dataset.codexPlusSidebarNavigationListener !== "true") {
      navigation.dataset.codexPlusSidebarNavigationListener = "true";
      navigation.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : event.target?.parentElement;
        if (target?.closest(`#${codexPlusSidebarNavId}`)) return;
        if (target?.closest("button, a")) closeCodexPlusPageAfterNativeNavigation();
      }, true);
    }
    let wrapper = document.getElementById(codexPlusSidebarNavId);
    const parent = insertionButton?.parentElement || navigation;
    if (!wrapper || wrapper.parentElement !== parent || !codexPlusNavEntryReusable(wrapper)) {
      wrapper?.remove();
      wrapper = document.createElement("div");
      wrapper.id = codexPlusSidebarNavId;
      wrapper.__codexPlusGeneration = codexPlusBackendGeneration;
      wrapper.dataset.codexPlusSidebarNav = "true";
      const button = (insertionButton || document.createElement("button")).cloneNode(true);
      if (!(button instanceof HTMLElement)) return;
      if (!button.className) button.className = "h-token-nav-row w-full flex items-center gap-2 px-3 py-2 text-sm";
      button.type = "button";
      button.removeAttribute("data-state");
      button.removeAttribute("data-sidebar-destination");
      button.removeAttribute("aria-current");
      button.removeAttribute("disabled");
      button.removeAttribute("aria-disabled");
      button.setAttribute("aria-label", "Codex++");
      button.textContent = "";
      button.innerHTML = `<span class="codex-plus-sidebar-nav-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18M3 12h18M5.5 5.5l13 13M18.5 5.5l-13 13"/></svg></span><span class="truncate">Codex++</span><span class="codex-plus-sidebar-nav-status" data-status="${codexPlusBackendStatus.status || "checking"}" aria-hidden="true"></span>`;
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        openCodexPlusPage();
      }, true);
      wrapper.appendChild(button);
      if (insertionButton?.nextSibling) {
        parent.insertBefore(wrapper, insertionButton.nextSibling);
      } else {
        parent.appendChild(wrapper);
      }
    }
    const status = wrapper.querySelector(".codex-plus-sidebar-nav-status");
    if (status) status.dataset.status = codexPlusBackendStatus.status || "checking";
    const active = !!document.querySelector(`.${codexPlusPageClass}`);
    setCodexPlusSidebarNavActive(active, codexPlusActiveEntry() || "home");
  }

  function removeCodexPlusRailNavigation() {
    [codexPlusRailNavId, codexPlusRailProviderId, codexPlusRailRemoteId, codexPlusRailWeixinId, codexPlusRailTasksId, codexPlusRailExtensionsId, codexPlusRailSponsorId, codexPlusRailPluginMarketId].forEach((id) => document.getElementById(id)?.remove());
  }

  function detachCodexPlusSidebarNavigation() {
    document.getElementById(codexPlusSidebarNavId)?.remove();
    document.getElementById(codexPlusSidebarPluginMarketId)?.remove();
  }

  /**
   * 挑一个原生 rail 按钮当模板。
   *
   * 优先 builtin:projects——它在 primary 区，且不像 builtin:library 那样会走
   * tooltip/triggerRef 的特殊分支。找不到就退回第一个可见 destination。
   */
  function codexPlusRailTemplateButton(rail) {
    const preferred = rail.querySelector(`${codexPlusRailDestinationSelector}[data-sidebar-destination="builtin:projects"]`);
    if (preferred) return preferred;
    const candidates = Array.from(rail.querySelectorAll(codexPlusRailDestinationSelector))
      .filter((node) => node.closest("nav") === rail);
    // 优先挑未选中的：clone 会把选中态的属性和配色一起带过来，
    // 表现为入口在没有任何页面打开时也显示成选中。
    const isSelected = (node) => node.getAttribute("aria-current") === "page" || node.hasAttribute("data-selected");
    return candidates.find((node) => !isSelected(node)) || candidates[0] || null;
  }

  function codexPlusRailPrimaryAnchor(rail) {
    const fixedIds = [
      'builtin:home',
      'builtin:customize',
    ];
    const buttons = Array.from(rail.querySelectorAll(codexPlusRailDestinationSelector));
    return buttons.find((node) => {
      const id = node.getAttribute("data-sidebar-destination") || "";
      return id && !fixedIds.includes(id);
    }) || null;
  }

  /**
   * 入口按钮能否沿用。
   *
   * 同页重复注入（Codex++ 重启 / 升级而 Codex 没重启）时，上一次注入建的入口按钮还在，
   * 按钮上绑的是旧闭包：点「拓展」打开的仍是旧版页面，里面的开关也走旧逻辑。
   * 按钮记下建它的注入代次；最新一代遇到别代的按钮就重建。旧代即便还在跑也不重建，
   * 免得两代来回抢同一个 id。
   */
  function codexPlusNavEntryReusable(node) {
    return node.__codexPlusGeneration === codexPlusBackendGeneration
      || codexPlusBackendGeneration !== window.__codexPlusBackendGeneration;
  }

  function createCodexPlusRailButton({ id, template, label, iconMarkup, withStatus, onActivate }) {
    const wrapper = document.createElement("div");
    wrapper.id = id;
    wrapper.__codexPlusGeneration = codexPlusBackendGeneration;
    wrapper.dataset.codexPlusRail = id === codexPlusRailExtensionsId ? "extensions" : id === codexPlusRailProviderId ? "provider" : "home";
    // 模板拿不到时不回退到旧模式，而是自建一个按钮：rail 上 destination 可能在
    // 登录态/接口就绪前还是空的，那只是暂时状态，不该让入口整个消失。
    const button = template
      ? template.cloneNode(true)
      : document.createElement("button");
    if (!(button instanceof HTMLElement)) return null;
    button.type = "button";
    // 留着 data-sidebar-destination 会被 Codex 的自定义/排序逻辑当成真的 destination。
    button.removeAttribute("data-sidebar-destination");
    // 留着悬浮区标记，侧栏收起时鼠标停在我们的入口上会浮出模板那一项（如「定时任务」）的原生侧栏。
    button.removeAttribute("data-slate-sidebar-peek-area");
    button.removeAttribute("data-slate-sidebar-peek-product-mode");
    button.removeAttribute("aria-expanded");
    button.removeAttribute("data-state");
    button.removeAttribute("disabled");
    button.removeAttribute("aria-disabled");
    // 选中态由 data-selected 驱动，但它只在没有 data-suppress-active-style 时生效。
    // 模板若是未选中的按钮，会带着 suppress 过来，压制掉我们的选中样式——
    // 必须移除，否则按钮永远停在未选中的暗色。
    button.removeAttribute("data-suppress-active-style");
    // 起始为未选中；激活态由 setCodexPlusSidebarNavActive 切换 data-selected。
    button.removeAttribute("data-selected");
    button.removeAttribute("aria-current");
    button.setAttribute("aria-label", label);
    // 不用 title：那会弹出系统自带的提示框，样式和位置都和原生按钮不一样。
    button.removeAttribute("title");
    attachCodexPlusRailTooltip(button, label);
    button.textContent = "";
    // 原生 rail 按钮是纯图标，没有文字标签，所以只放图标 + 状态点。
    button.innerHTML = `<span class="codex-plus-rail-icon" aria-hidden="true">${iconMarkup}</span>`
      + (withStatus
        ? `<span class="codex-plus-sidebar-nav-status" data-status="${codexPlusBackendStatus.status || "checking"}" aria-hidden="true"></span>`
        : "");
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      onActivate();
    }, true);
    wrapper.appendChild(button);
    return wrapper;
  }

  /**
   * 图标栏按钮的悬浮提示，外观与行为对齐原生按钮。
   *
   * 原生按钮的提示由 React（Radix Tooltip）渲染，我们复制出来的按钮接不上它，
   * 所以自己画：直接复用原生提示的样式类（深浅主题、版本更新都跟着走），找不到时
   * 用量出来的数值兜底。位置与时机照原生：按钮右侧 2px、垂直居中；悬停约 150ms
   * 出现，在相邻按钮间移动时（300ms 内）立即出现；按下、移出、失焦即消失。
   */
  const codexPlusRailTooltipDelayMs = 150;
  const codexPlusRailTooltipSkipDelayMs = 300;
  const codexPlusRailTooltipSideOffset = 2;
  const codexPlusComposerTooltipSideOffset = 6;
  let codexPlusRailTooltipTimer = null;
  let codexPlusRailTooltipHiddenAt = 0;
  let codexPlusRailTooltipOwner = null;
  let codexPlusNativeTooltipClassCache;

  /**
   * 原生提示的样式类名带构建哈希（如 _Tooltip_11i4f_2），每个版本都可能变，只能运行时找。
   * 它独有一条 `[data-side="bottom"][data-default-side-offset="true"]` 变体，用来和
   * 其它同名前缀的组件区分开。规则可能嵌在 @layer / @supports 里，要递归找。
   */
  function codexPlusNativeTooltipClass(sheets = document.styleSheets) {
    if (codexPlusNativeTooltipClassCache !== undefined) return codexPlusNativeTooltipClassCache;
    const signature = /^\.(_Tooltip_[\w-]+)\[data-side="bottom"\]\[data-default-side-offset="true"\]$/;
    let found = "";
    const walk = (rules) => {
      for (const rule of rules || []) {
        if (found) return;
        const match = signature.exec(rule.selectorText || "");
        if (match) found = match[1];
        else if (rule.cssRules) walk(rule.cssRules);
      }
    };
    for (const sheet of sheets || []) {
      try {
        walk(sheet.cssRules);
      } catch {}
      if (found) break;
    }
    // 只缓存找到的结果：样式表可能晚于第一次悬停才加载完。
    if (found) codexPlusNativeTooltipClassCache = found;
    return found;
  }

  function hideCodexPlusRailTooltip() {
    clearTimeout(codexPlusRailTooltipTimer);
    codexPlusRailTooltipTimer = null;
    const node = document.getElementById("codex-plus-rail-tooltip");
    if (node) {
      node.remove();
      codexPlusRailTooltipHiddenAt = Date.now();
    }
    codexPlusRailTooltipOwner = null;
  }

  function showCodexPlusRailTooltip(button, label, side = "right") {
    if (!button.isConnected || codexPlusRailTooltipOwner !== button) return;
    document.getElementById("codex-plus-rail-tooltip")?.remove();
    const nativeClass = codexPlusNativeTooltipClass();
    const zoom = codexPlusWindowZoom();
    const wrapper = document.createElement("div");
    wrapper.id = "codex-plus-rail-tooltip";
    markCodexPlusExtensionNode(wrapper, "builtin-rail-tooltip");
    // 挂在 body 下、不在 Codex 的缩放容器里，所以自己套上界面缩放，坐标相应除回去。
    wrapper.style.cssText = `position:fixed;left:0;top:0;z-index:2147483645;pointer-events:none;zoom:${zoom};`;
    const tip = document.createElement("div");
    tip.setAttribute("role", "tooltip");
    tip.className = nativeClass || "codex-plus-rail-tooltip-fallback";
    Object.assign(tip.dataset, { side, align: "center", state: "delayed-open", gutterSize: "md", defaultSideOffset: "true", clickable: "false" });
    tip.textContent = label;
    wrapper.appendChild(tip);
    document.body.appendChild(wrapper);
    const anchor = button.getBoundingClientRect();
    const box = tip.getBoundingClientRect();
    // 图标栏的提示在右侧，输入区按钮的提示在上方，标题栏与消息操作行的在下方（与原生对应位置的按钮一致）。
    // 上下方向的提示按窗口宽度收进来：贴着窗口右边的按钮不能把提示挤出窗外。
    const vertical = side === "top" || side === "bottom";
    const left = vertical
      ? Math.min(Math.max(4, anchor.left + (anchor.width - box.width) / 2), window.innerWidth - box.width - 4) / zoom
      : (anchor.right + codexPlusRailTooltipSideOffset) / zoom;
    const top = side === "top"
      ? (anchor.top - box.height - codexPlusComposerTooltipSideOffset) / zoom
      : side === "bottom"
        ? (anchor.bottom + codexPlusComposerTooltipSideOffset) / zoom
        : (anchor.top + (anchor.height - box.height) / 2) / zoom;
    wrapper.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
    button.setAttribute("aria-describedby", wrapper.id);
  }

  /**
   * 窗口失焦、滚动时收起提示。按钮所在的 rail 被 Codex 重建时不会再收到 pointerleave，
   * 这两个全局时机兜底。同页重复注入时先摘掉旧监听。
   */
  function installCodexPlusRailTooltipDismiss() {
    if (window.__codexPlusRailTooltipDismiss === hideCodexPlusRailTooltip) return;
    window.removeEventListener("blur", window.__codexPlusRailTooltipDismiss);
    document.removeEventListener("scroll", window.__codexPlusRailTooltipDismiss, true);
    window.__codexPlusRailTooltipDismiss = hideCodexPlusRailTooltip;
    window.addEventListener("blur", hideCodexPlusRailTooltip);
    document.addEventListener("scroll", hideCodexPlusRailTooltip, true);
  }

  function attachCodexPlusRailTooltip(button, label, side = "right") {
    installCodexPlusRailTooltipDismiss();
    const open = () => {
      clearTimeout(codexPlusRailTooltipTimer);
      codexPlusRailTooltipOwner = button;
      const instant = Date.now() - codexPlusRailTooltipHiddenAt < codexPlusRailTooltipSkipDelayMs
        || !!document.getElementById("codex-plus-rail-tooltip");
      if (instant) showCodexPlusRailTooltip(button, label, side);
      else codexPlusRailTooltipTimer = setTimeout(() => showCodexPlusRailTooltip(button, label, side), codexPlusRailTooltipDelayMs);
    };
    const close = () => {
      if (codexPlusRailTooltipOwner !== button) return;
      button.removeAttribute("aria-describedby");
      hideCodexPlusRailTooltip();
    };
    button.addEventListener("pointerenter", open);
    button.addEventListener("pointerleave", close);
    button.addEventListener("focus", () => {
      if (button.matches?.(":focus-visible")) open();
    });
    button.addEventListener("blur", close);
    // 按下就收起，和原生一致；点击后页面切换，提示不该留在原处。
    button.addEventListener("pointerdown", close);
  }

  /** Codex++ 自己各页面的图标：图标栏的 Codex++ 入口用 logo，收进去的页面在左栏「页面」一节里用其余几个。 */
  function codexPlusPageIcons() {
    return {
      // Codex++ 入口直接用 Codex++ 自己的 logo（彩色），一眼能和原生入口区分开。
      home: `<img class="codex-plus-rail-logo" src="${codexPlusLogoDataUrl}" alt="" draggable="false">`,
      // 「拓展」直接用 VSCode 的扩展字形（就是列表里默认图标那一份），
      // 和页面内部保持同一个符号，不再另画一个近似图形。
      extensions: `<svg viewBox="0 0 16 16" fill="currentColor"><path d="${codexPlusDefaultExtensionIconPath}"/></svg>`,
      // Lucide 的 server：同一套 24 格线性风格。
      provider: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="7" rx="2"/><rect x="3" y="14" width="18" height="7" rx="2"/><path d="M7 6.5h.01M7 17.5h.01"/></svg>',
      // Lucide 的 megaphone：同一套 24 格线性风格。
      sponsor: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m3 11 18-5v12L3 14v-3z"/><path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"/></svg>',
      // Lucide 的 smartphone：同一套 24 格线性风格。
      remote: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="2" width="14" height="20" rx="2"/><path d="M12 18h.01"/></svg>',
      // Lucide 的 message-circle：同一套 24 格线性风格（管理器微信页的状态图标也是它）。
      weixin: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/></svg>',
      // Lucide 的 square-kanban：同一套 24 格线性风格。
      tasks: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M8 7v7M12 7v4M16 7v9"/></svg>',
    };
  }

  /**
   * 收进 Codex++ 入口的页面：不占图标栏的图标，列在 Codex++ 左栏（和侧栏收起时浮出的卡片）最上面的「页面」一节，
   * 点开后仍在 Codex++ 页面里面显示（installCodexPlusEmbeddedNav）。entry 与 codexPlusActiveEntry 的取值一致。
   * 远程控制与微信连接相邻：两者都是「在手机上用 Codex」，都由管理器托管、Codex 没开也能用。
   * 任务面板、拓展、推荐内容仍各占一个图标（installCodexPlusRailNavigation）。
   */
  function codexPlusFoldedPages() {
    const icons = codexPlusPageIcons();
    return [
      { entry: "provider", label: "供应商", icon: icons.provider, open: () => openCodexPlusProviderPage() },
      { entry: "remote", label: "远程控制", icon: icons.remote, tagHtml: '<em class="codex-plus-feature-tag" data-tone="experimental">实验</em>', open: () => openCodexPlusRemoteControlPage() },
      { entry: "weixin", label: "微信连接", icon: icons.weixin, open: () => openCodexPlusWeixinConnectPage() },
    ];
  }

  function openCodexPlusFoldedPage(entry) {
    codexPlusFoldedPages().find((page) => page.entry === entry)?.open();
  }

  /**
   * 把 Codex++、任务面板、拓展、推荐内容四个入口挂到新版图标栏（收进 Codex++ 里面的页面见 codexPlusFoldedPages）。
   *
   * Codex 的 rail 渲染晚于注入，所以这里每次 scan 都会被调用；靠 id 判存避免重复插入。
   */
  function installCodexPlusRailNavigation() {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => node.remove());
    const rail = document.querySelector(codexPlusRailSelector);
    if (!rail) return false;
    // 注意：模板按钮可能在 rail 还没渲染出 destination 时拿不到（登录态/接口未就绪）。
    // 那只是暂时状态，不能因此判定"没有 rail"而回退旧模式，否则入口会整个消失。
    const template = codexPlusRailTemplateButton(rail);

    // 旧逻辑把"点原生导航就关掉 Codex++ 页面"的监听挂在侧边栏的 navigation 上，
    // 但 rail 模式下那个函数会提前 return，监听压根装不上，所以这里补一份。
    if (rail.dataset.codexPlusRailNavigationListener !== "true") {
      rail.dataset.codexPlusRailNavigationListener = "true";
      rail.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : event.target?.parentElement;
        if (target?.closest(`#${codexPlusRailNavId}, #${codexPlusRailProviderId}, #${codexPlusRailExtensionsId}, #${codexPlusRailSponsorId}, #${codexPlusRailRemoteId}, #${codexPlusRailWeixinId}, #${codexPlusRailTasksId}`)) return;
        // 拓展入口的 id 是动态生成的，不在上面三个之内。不排除它，点拓展入口会被
        // 当成「点了原生导航按钮」，刚打开的拓展页面立刻被关掉。
        if (target?.closest(`[${codexPlusExtensionConstants.extensionAttribute}]`)) return;
        if (target?.closest("button, a")) closeCodexPlusPageAfterNativeNavigation();
      }, true);
    }

    const icons = codexPlusPageIcons();
    const specs = [
      { id: codexPlusRailNavId, label: "Codex++", iconMarkup: icons.home, withStatus: true, onActivate: openCodexPlusPage },
      { id: codexPlusRailTasksId, label: "任务面板", iconMarkup: icons.tasks, withStatus: false, onActivate: openCodexPlusTaskBoardPage },
      { id: codexPlusRailExtensionsId, label: "拓展", iconMarkup: icons.extensions, withStatus: false, onActivate: openCodexPlusExtensions },
      { id: codexPlusRailSponsorId, label: "推荐内容", iconMarkup: icons.sponsor, withStatus: false, onActivate: openCodexPlusSponsor },
    ];
    // 供应商、远程控制、微信连接已经收进 Codex++：旧注入留下的入口摘掉。
    [codexPlusRailProviderId, codexPlusRailRemoteId, codexPlusRailWeixinId]
      .forEach((id) => document.getElementById(id)?.remove());

    const anchor = codexPlusRailPrimaryAnchor(rail);
    // 插到锚点所在的父容器里，而不是 nav 顶层：原生按钮可能嵌在 nav 内部的分组 div 中，
    // 直接插顶层会破坏它的 flex 布局。
    const host = anchor?.parentElement || rail;
    let cursor = anchor;
    specs.forEach((spec) => {
      let wrapper = document.getElementById(spec.id);
      if (!wrapper || wrapper.parentElement !== host || !codexPlusNavEntryReusable(wrapper)) {
        wrapper?.remove();
        wrapper = createCodexPlusRailButton({ ...spec, template });
        if (!wrapper) return;
      }
      // 顺序：Codex++ 在前，「拓展」在后；紧跟在 primary 区锚点后面。
      if (cursor?.nextSibling) {
        host.insertBefore(wrapper, cursor.nextSibling);
      } else if (cursor) {
        host.appendChild(wrapper);
      } else {
        host.insertBefore(wrapper, host.firstElementChild);
      }
      cursor = wrapper;
    });

    const status = document.getElementById(codexPlusRailNavId)?.querySelector(".codex-plus-sidebar-nav-status");
    if (status) status.dataset.status = codexPlusBackendStatus.status || "checking";
    // 侧栏收起时入口浮出各自的侧栏卡片，原生页面上也要有，所以随入口一起装。
    installCodexPlusNavPeek();
    // 每轮扫描顺带检查开着的页面量的是不是现在这批外壳节点（Codex 重建外壳后要重量位置）。
    refreshCodexPlusPagePosition();
    return true;
  }

  /** 图标栏存在时走它，否则回退到旧版宽面板侧边栏入口。两条路径互斥，不会重复出现。 */
