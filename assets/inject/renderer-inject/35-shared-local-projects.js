  // Companion to codex_app_state's existing offline account-scope reconciliation.
  // Observe native notifications; never replace native services or account selection.
  function codexLocalProjectServicesFromModule(module) {
    const candidates = Object.values(module || {}).filter((value) => {
      if (!value || typeof value !== "object") return false;
      try {
        return ["applicationState", "projects", "threadProjectAssignments", "executionPaths"]
          .every((name) => Object.prototype.hasOwnProperty.call(value, name));
      } catch { return false; }
    });
    const unique = [...new Set(candidates)];
    return unique.length === 1 ? unique[0] : null;
  }

  async function loadCodexLocalProjectServices({
    urls = Array.from(document.scripts || []).map((script) => script.src),
    importModule = (url) => import(url),
    legacy = () => codexHostServices(),
  } = {}) {
    const services = new Set();
    for (const url of [...new Set(urls)].filter((url) => /\/assets\/\d+\.[a-f\d]+\.js(?:\?.*)?$/.test(url))) {
      let runtime;
      try { runtime = (await importModule(url)).__webpack_require__; } catch { continue; }
      if (!runtime?.c) continue;
      // Inspect only executed modules: invoking a factory creates duplicate RPC clients.
      for (const module of Object.values(runtime.c)) {
        try {
          const candidate = codexLocalProjectServicesFromModule(module?.exports);
          if (candidate) services.add(candidate);
        } catch {}
      }
    }
    if (services.size > 1) throw Error("Ambiguous native local project services");
    if (services.size === 1) return [...services][0];
    const candidate = codexLocalProjectServicesFromModule({ services: await legacy() });
    if (!candidate) throw Error("Native local project services unavailable");
    return candidate;
  }

  function createCodexSharedLocalProjects({ services, locks, enabled = () => true, onError = () => {} }) {
    const key = "codex-plus-shared-local-projects-v1";
    const ownerKey = "electron-account-renderer-state-v1";
    const own = (value, name) => Object.prototype.hasOwnProperty.call(value, name);
    const plain = (value) => value && typeof value === "object" && !Array.isArray(value);
    const clone = (value) => JSON.parse(JSON.stringify(value));
    const sorted = (value) => Array.isArray(value) ? value.map(sorted) : plain(value)
      ? Object.fromEntries(Object.keys(value).sort().map((name) => [name, sorted(value[name])])) : value;
    const same = (a, b) => JSON.stringify(sorted(a)) === JSON.stringify(sorted(b));
    const id = (value) => typeof value === "string" && value.length > 0 && value.length <= 200
      && !["__proto__", "constructor", "prototype"].includes(value) && !value.startsWith("linked:");
    const normalize = (value) => {
      if (typeof value !== "string" || !value) throw Error("Local project home unavailable");
      const path = value.replace(/\\/g, "/").replace(/^\/\/\?\//, "").replace(/\/+$/, "") || "/";
      return /^[a-z]:|^\/\//i.test(path) ? path.toLowerCase() : path;
    };
    const project = (value, name) => {
      if (!plain(value) || value.id !== name || !id(name)
          || typeof value.name !== "string" || !value.name.trim()
          || !Array.isArray(value.rootPaths) || !value.rootPaths.length
          || value.rootPaths.some((path) => typeof path !== "string"
            || !/^(?:[a-z]:[\\/]|\/|\\\\)/i.test(path) || /:\/\//.test(path))
          || !Number.isFinite(value.createdAt) || !Number.isFinite(value.updatedAt)
          || Object.keys(value).some((name) => /chatgpt|backing|remote|durable|origin/i.test(name)
            && value[name] != null)) return null;
      return { id: name, name: value.name, rootPaths: [...value.rootPaths],
        createdAt: value.createdAt, updatedAt: value.updatedAt };
    };
    const projects = (values) => Object.fromEntries(Object.entries(plain(values) ? values : {})
      .flatMap(([name, value]) => { const safe = project(value, name); return safe ? [[name, safe]] : []; }));
    const sameProject = (a, b) => a == null || b == null ? a === b
      : a.name === b.name && same(a.rootPaths, b.rootPaths);
    const state = services.applicationState;
    const read = async (name) => {
      const result = await state.read(name);
      if (!plain(result) || !own(result, "value")) throw Error("Unsupported native project state");
      return result.value;
    };
    const owner = async () => {
      const value = await read(ownerKey);
      if (value == null) return null;
      if (!plain(value) || !(value.owner === null || typeof value.owner === "string") || !plain(value.inactive))
        throw Error("Unsupported native account scope");
      return value.owner;
    };
    let home, stable, flight, timer, startupRetry, visible = new Set();
    let epoch = 0, stopped = false, pending = false, restoring = false, initial = true, retries = 0;
    let startupAttempts = 0;
    let orderChangedDuringRestore = false;
    const events = [];
    const expected = [];
    const active = () => !stopped && enabled();
    const cancelled = () => Object.assign(Error("Native account scope changed"), { name: "CodexSharedScopeChanged" });
    const check = async (token) => {
      if (!active() || token.epoch !== epoch || token.owner !== await owner()) throw cancelled();
    };
    const native = async (token, operation) => {
      await check(token);
      const result = await operation();
      await check(token);
      return result;
    };
    const viewFromGlobals = (globals) => ({
      raw: globals["local-projects"] || {}, projects: projects(globals["local-projects"]),
      assignments: globals["thread-project-assignments"] || {},
      projectless: globals["projectless-thread-ids"] || [],
      hosts: globals["thread-project-membership-host-ids"] || {}, order: globals["project-order"] || [],
    });
    const view = async (token) => native(token, async () => {
      const [raw, memberships, order, hosts] = await Promise.all([
        services.projects.getLocalProjects(), services.threadProjectAssignments.readMemberships(),
        read("project-order"), read("thread-project-membership-host-ids"),
      ]);
      if (!plain(raw) || !plain(memberships?.assignments) || !Array.isArray(memberships.projectlessThreadIds)
          || (order != null && !Array.isArray(order)) || (hosts != null && !plain(hosts)))
        throw Error("Unsupported native project metadata");
      return { raw, projects: projects(raw), assignments: memberships.assignments,
        projectless: memberships.projectlessThreadIds, hosts: hosts || {}, order: order || [] };
    });
    const membership = (current, threadId, known = false) => {
      if (!id(threadId) || (current.hosts[threadId] != null && current.hosts[threadId] !== "local")) return null;
      const assignment = current.assignments[threadId];
      if (assignment != null) {
        if (assignment.projectKind !== "local" || assignment.projectOrigin != null
            || !own(current.projects, assignment.projectId)) return null;
        return { threadId, hostId: "local", assignment: { projectKind: "local", projectId: assignment.projectId },
          projectless: false };
      }
      return known ? { threadId, hostId: "local", assignment: null,
        projectless: current.projectless.includes(threadId) } : null;
    };
    const save = async (token, record) => {
      const next = { ...record, revision: record.revision + 1 };
      if (JSON.stringify(next).length > 4 * 1024 * 1024) throw Error("Shared project metadata is too large");
      await native(token, async () => {
        if ((await state.write(key, next))?.success !== true) throw Error("Shared project save was refused");
      });
      return next;
    };
    const load = async (token, current) => {
      const saved = await native(token, () => read(key));
      if (saved != null) {
        if (!plain(saved) || saved.version !== 1 || saved.home !== home
            || !Number.isSafeInteger(saved.revision) || saved.revision < 1 || saved.revision >= Number.MAX_SAFE_INTEGER
            || !plain(saved.projects) || !plain(saved.memberships) || !Array.isArray(saved.knownProjects)
            || !Array.isArray(saved.order) || saved.knownProjects.some((name) => !id(name))
            || Object.keys(saved.projects).some((name) => !saved.knownProjects.includes(name))
            || !same(projects(saved.projects), saved.projects)
            || Object.entries(saved.memberships).some(([threadId, entry]) => !id(threadId)
              || entry?.threadId !== threadId || entry.hostId !== "local" || typeof entry.projectless !== "boolean"
              || (entry.assignment !== null && (entry.assignment?.projectKind !== "local"
                || !own(saved.projects, entry.assignment.projectId)))))
          throw Object.assign(Error("Unsupported shared local project record"), { name: "CodexSharedRecordUnsupported" });
        return saved;
      }
      const container = await native(token, () => read(ownerKey));
      const seed = { version: 1, home, revision: 0, enabled: true, projects: {}, memberships: {}, knownProjects: [], order: [] };
      const sources = Object.values(container?.inactive || {}).map((scope) => {
        if (!plain(scope?.globals)) throw Error("Unsupported inactive local projects");
        return viewFromGlobals(scope.globals);
      });
      for (const source of [...sources, current]) {
        if (!plain(source.assignments) || !plain(source.hosts) || !Array.isArray(source.projectless)
            || !Array.isArray(source.order)) throw Error("Unsupported inactive project classification");
        for (const [name, value] of Object.entries(source.projects))
          if (source === current || !own(seed.projects, name) || value.updatedAt >= seed.projects[name].updatedAt)
            seed.projects[name] = value;
        for (const threadId of Object.keys(source.assignments)) {
          const entry = membership(source, threadId);
          if (entry) seed.memberships[threadId] = entry;
          else if (source === current) delete seed.memberships[threadId];
        }
        for (const threadId of source.projectless) {
          if ((source.hosts[threadId] !== "local" && !own(seed.memberships, threadId)) || source.assignments[threadId]) continue;
          const entry = membership(source, threadId, true);
          if (entry) seed.memberships[threadId] = entry;
        }
        if (source === current) for (const threadId of Object.keys(seed.memberships)) {
          if (source.hosts[threadId] != null && source.hosts[threadId] !== "local") delete seed.memberships[threadId];
          else if (!source.assignments[threadId] && source.hosts[threadId] === "local")
            seed.memberships[threadId] = membership(source, threadId, true);
        }
        seed.order = [...new Set([...source.order.filter((name) => own(seed.projects, name)), ...seed.order])];
      }
      for (const [threadId, entry] of Object.entries(seed.memberships))
        if (entry.assignment && !own(seed.projects, entry.assignment.projectId)) delete seed.memberships[threadId];
      seed.knownProjects = Object.keys(seed.projects);
      seed.order = seed.order.filter((name) => own(seed.projects, name));
      seed.backup = clone({ projects: seed.projects, memberships: seed.memberships, order: seed.order });
      return save(token, seed);
    };
    const capture = (record, current, messages, terminal = false) => {
      const next = clone(record);
      const projectEvent = terminal || messages.some((e) => e.type === "global-state-updated" && e.keys?.includes("local-projects"));
      if (projectEvent) {
        for (const [name, value] of Object.entries(current.projects)) {
          if (terminal && next.knownProjects.includes(name) && !own(next.projects, name)) continue;
          next.projects[name] = value;
          if (!next.knownProjects.includes(name)) next.knownProjects.push(name);
        }
        if (terminal)
          for (const name of record.lastScope.projects)
            if (!own(current.raw, name)) delete next.projects[name];
        for (const message of messages)
          for (const name of message.deletedProjects || [])
            if (next.knownProjects.includes(name)) delete next.projects[name];
      }
      if (terminal || messages.some((e) => e.type === "global-state-updated" && e.keys?.includes("project-order")))
        next.order = current.order.filter((name) => own(next.projects, name));
      const threadIds = new Set(terminal
        ? [...record.lastScope.memberships, ...Object.keys(current.assignments).filter((id) => membership(current, id))]
        : messages.filter((e) => e.type === "thread-project-memberships-updated").flatMap((e) => e.memberships.map((m) => m.threadId)));
      for (const threadId of threadIds) {
        const entry = membership(current, threadId, own(next.memberships, threadId));
        if (entry) next.memberships[threadId] = entry;
        else delete next.memberships[threadId];
      }
      for (const [threadId, entry] of Object.entries(next.memberships))
        if (entry.assignment && !own(next.projects, entry.assignment.projectId))
          next.memberships[threadId] = { threadId, hostId: "local", assignment: null, projectless: true };
      return next;
    };
    const writeMemberships = async (token, records) => {
      if (restoring) expected.push(...records.map(clone));
      return native(token, () => services.threadProjectAssignments.restoreMemberships(records));
    };
    const restore = async (token, record, current) => {
      restoring = true;
      orderChangedDuringRestore = false;
      visible = new Set(Object.keys(current.projects));
      // Upsert establishes the app-server ID mapping as well as the visible project.
      const upserts = Object.keys(record.projects).filter((name) => !(own(current.raw, name) && !own(current.projects, name))
        && (initial || !sameProject(current.projects[name], record.projects[name])));
      const protectedThreads = Object.values(record.memberships).filter((entry) =>
        (entry.assignment ? upserts.includes(entry.assignment.projectId) : upserts.length > 0)
        && !current.assignments[entry.threadId] && !current.projectless.includes(entry.threadId)
        && (current.hosts[entry.threadId] == null || current.hosts[entry.threadId] === "local"))
        .map((entry) => ({ threadId: entry.threadId, hostId: "local", assignment: null, projectless: true }));
      let completed = false;
      try {
        if (protectedThreads.length) await writeMemberships(token, protectedThreads);
        for (const name of upserts) {
          const value = record.projects[name];
          const latest = await native(token, () => services.projects.getLocalProjects());
          if (project(latest[name], name) && !sameProject(latest[name], current.projects[name])
              && !sameProject(latest[name], value)) continue;
          visible.add(name);
          if (await native(token, () => services.projects.upsertLocal({
            projectId: name, name: value.name, sources: value.rootPaths,
          }, "restore")) !== true) throw Error("Native project restore was refused");
        }
        for (const name of record.knownProjects)
          if (!own(record.projects, name) && own(current.projects, name))
            await native(token, () => services.projects.removeLocal(name));
        current = await view(token);
        const order = [...new Set([...record.order.filter((name) => own(current.projects, name)),
          ...current.order.filter((name) => !record.knownProjects.includes(name))])];
        if (!orderChangedDuringRestore && !same(order, current.order))
          await native(token, () => services.projects.setProjectOrder(order));
        const changed = Object.values(record.memberships).filter((entry) => {
          if (current.hosts[entry.threadId] != null && current.hosts[entry.threadId] !== "local") return false;
          const existing = current.assignments[entry.threadId];
          if (existing && !membership(current, entry.threadId)) return false;
          if (entry.assignment && !own(current.projects, entry.assignment.projectId)) return false;
          return !same(existing || null, entry.assignment) || current.projectless.includes(entry.threadId) !== entry.projectless;
        });
        if (changed.length) await writeMemberships(token, changed);
        completed = true;
        initial = false;
      } finally {
        restoring = false;
        expected.length = 0;
        if (!completed && protectedThreads.length && active() && token.epoch === epoch && token.owner === await owner()) {
          const current = await services.threadProjectAssignments.readMemberships();
          const rollback = protectedThreads.filter((entry) => !current.assignments[entry.threadId]
            && current.projectlessThreadIds.includes(entry.threadId)).map((entry) => ({ ...entry, projectless: false }));
          if (rollback.length && token.epoch === epoch && token.owner === await owner())
            await writeMemberships(token, rollback);
        }
      }
    };
    const reconcile = () => {
      if (!active()) return Promise.resolve();
      pending = true;
      if (flight) return flight;
      flight = (async () => {
        if (!locks?.request) throw Error("Cross-window project lock unavailable");
        home ||= normalize(await services.executionPaths.readCodexHome("local"));
        let budget = 4;
        while (pending && active() && budget-- > 0) {
          pending = false;
          await locks.request(`codex-plus-local-projects:${home}`, async () => {
            if (!active()) return;
            if (normalize(await services.executionPaths.readCodexHome("local")) !== home) throw Error("Local project home changed");
            const token = { epoch, owner: await owner() };
            let current = await view(token);
            let record = await load(token, current);
            const messages = events.splice(0);
            // A full atom sync also happens when opening an overlay, without an
            // account change. Only the matching empty project reset needs recovery.
            const reset = messages.some((e) => e.type === "persisted-atom-sync")
              && !messages.some((e) => e.deletionSnapshot)
              && !Object.keys(current.raw).length && !Object.keys(current.assignments).length
              && !current.projectless.length && !current.order.length;
            const changedScope = !stable || stable.owner !== token.owner || stable.epoch !== epoch || reset;
            if (initial && record.preparedForLaunch !== true && record.lastScope?.owner != null && id(record.lastScope.owner)
                && Array.isArray(record.lastScope.projects) && Array.isArray(record.lastScope.memberships)) {
              const container = await native(token, () => read(ownerKey));
              const finalScope = container?.inactive?.[record.lastScope.owner]?.globals;
              if (plain(finalScope)) record = await save(token, capture(record, viewFromGlobals(finalScope), [], true));
            }
            if (!changedScope && messages.length) {
              const replay = messages.filter((e) => e.duringRestore).flatMap((e) => e.memberships || [])
                .filter((entry) => id(entry.threadId)
                  && (current.hosts[entry.threadId] == null || current.hosts[entry.threadId] === "local")
                  && (!current.assignments[entry.threadId] || membership(current, entry.threadId))
                  && (entry.assignment == null ? own(record.memberships, entry.threadId)
                    : entry.assignment.projectKind === "local" && entry.assignment.projectOrigin == null
                      && own(current.projects, entry.assignment.projectId)));
              if (replay.length) {
                await writeMemberships(token, replay.map((entry) => ({ ...entry, hostId: "local" })));
                current = await view(token);
              }
              for (const event of messages) {
                if (!event.deletionSnapshot) continue;
                event.deletedProjects = await event.deletionSnapshot;
                await check(token);
              }
              const next = capture(record, current, messages);
              if (!same(next, record)) record = await save(token, next);
            }
            // A shared-record notification from another window also needs projection.
            if (changedScope || messages.some((e) => e.keys?.includes(key))) await restore(token, record, current);
            const verified = await view(token);
            const lastScope = { owner: token.owner,
              projects: Object.keys(record.projects).filter((id) => own(verified.projects, id)),
              memberships: Object.keys(record.memberships).filter((id) => membership(verified, id, true)) };
            if (!same(record.lastScope, lastScope) || record.preparedForLaunch === true || record.enabled !== true)
              record = await save(token, { ...record, lastScope, preparedForLaunch: false, enabled: true });
            stable = { owner: token.owner, epoch };
            visible = new Set(Object.keys(verified.projects));
          });
        }
      })().catch((error) => {
        if (error?.name === "CodexSharedScopeChanged") pending = true;
        else {
          onError(error);
          if (initial && error?.name !== "CodexSharedRecordUnsupported" && startupAttempts++ < 8)
            startupRetry = setTimeout(() => { startupRetry = null; void reconcile(); }, Math.min(5000, 100 * 2 ** startupAttempts));
        }
      }).finally(() => {
        flight = null;
        if (pending && active() && retries++ < 4) queueMicrotask(() => void reconcile());
        else if (!pending) retries = 0;
      });
      return flight;
    };
    return {
      key, reconcile,
      resume() { stable = null; epoch++; return reconcile(); },
      notify(message) {
        if (!active()) return;
        if (message?.type === "persisted-atom-sync") {
          events.push({ type: message.type });
          pending = true;
        } else if (message?.type === "thread-project-memberships-updated") {
          const entries = Array.isArray(message.memberships) ? message.memberships
            : Object.entries(plain(message.memberships) ? message.memberships : {}).map(([threadId, value]) => ({ ...value, threadId }));
          const memberships = entries.filter((entry) => {
            if (!restoring) return true;
            const index = expected.findIndex((e) => e.threadId === entry.threadId
              && same(e.assignment, entry.assignment ?? null) && e.projectless === entry.projectless);
            if (index < 0) return true;
            expected.splice(index, 1);
            return false;
          });
          if (memberships.length) events.push({ ...message, memberships, duringRestore: restoring });
        } else if (message?.type === "global-state-updated"
            && message.keys?.some((name) => [key, ownerKey, "local-projects", "project-order", "thread-project-assignments"].includes(name))) {
          const event = clone(message);
          if (restoring && message.keys.includes("project-order")) orderChangedDuringRestore = true;
          if (["local-projects", "project-order", "pinned-project-ids"].every((name) => message.keys.includes(name))) {
            const seen = [...visible], at = epoch;
            // Read at notification time, before a restore can re-add a concurrently deleted project.
            event.deletionSnapshot = (async () => {
              const before = await owner();
              const raw = await services.projects.getLocalProjects();
              if (at !== epoch || before !== await owner() || !plain(raw)) return [];
              return seen.filter((name) => !own(raw, name));
            })().catch(() => []);
          }
          events.push(event);
        } else return;
        if (timer == null) timer = setTimeout(() => { timer = null; void reconcile(); }, 0);
      },
      dispose() {
        stopped = true; epoch++;
        if (timer != null) clearTimeout(timer);
        if (startupRetry != null) clearTimeout(startupRetry);
      },
    };
  }

  function installCodexSharedLocalProjects() {
    window.__codexPlusSharedLocalProjects?.dispose();
    let runtime, starting, retry, disposed = false, attempts = 0;
    const enabled = () => !disposed && codexPlusBackendSettingsLoaded === true
      && codexPlusBackendSettings.keepProjectsWithoutAccount === true;
    const sync = () => {
      if (!enabled()) { runtime?.dispose(); runtime = null; return Promise.resolve(); }
      if (runtime) return runtime.reconcile();
      if (starting) return starting;
      starting = loadCodexLocalProjectServices().then(async (services) => {
        if (!enabled()) return;
        runtime = createCodexSharedLocalProjects({
          services, locks: navigator.locks, enabled,
          onError: () => sendCodexPlusDiagnostic("shared_local_projects_unavailable", { reason: "native-service-unavailable" }),
        });
        await runtime.reconcile();
      }).catch(() => {
        if (!enabled() || attempts++ >= 8) return;
        retry = setTimeout(() => { retry = null; void sync(); }, Math.min(5000, 250 * 2 ** attempts));
      }).finally(() => { starting = null; });
      return starting;
    };
    const incoming = (event) => {
      if (event.source != null && event.source !== window) return;
      if (event.origin && event.origin !== "null" && event.origin !== window.location.origin) return;
      runtime?.notify(event.data);
    };
    const handle = codexPlusCreateExtension({
      id: "internal:shared-local-projects",
      apply(ctx) {
        ctx.on("settings/loaded", () => void sync());
        ctx.on("settings/changed", () => void sync());
        ctx.effect(() => {
          window.addEventListener("message", incoming);
          return () => {
            disposed = true;
            if (retry != null) clearTimeout(retry);
            window.removeEventListener("message", incoming);
            runtime?.dispose();
          };
        });
      },
    }, { scriptKey: "internal:shared-local-projects" });
    window.__codexPlusSharedLocalProjects = { dispose: () => handle.dispose() };
    void sync();
  }
