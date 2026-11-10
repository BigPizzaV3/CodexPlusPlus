  // Keep only ordinary local project metadata in an unscoped, per-home record.
  // Native account selection, atoms, cloud projects and pending messages stay native.
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
    // New Rspack builds have numeric assets. Inspect only modules already executed
    // by the native page; running a factory again would create a second RPC client.
    const services = new Set();
    for (const url of [...new Set(urls)].filter((url) => /\/assets\/\d+\.[a-f\d]+\.js(?:\?.*)?$/.test(url))) {
      let runtime;
      try { runtime = (await importModule(url)).__webpack_require__; } catch { continue; }
      if (!runtime?.c) continue;
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

  function createCodexLocalProjectContinuity({ services, locks, enabled = () => true, onError = () => {}, onReady = () => {} }) {
    const key = "codex-plus-shared-local-projects-v1";
    const ownerKey = "electron-account-renderer-state-v1";
    const projectKey = "local-projects";
    const membershipKey = "thread-project-assignments";
    const plain = (value) => value && typeof value === "object" && !Array.isArray(value);
    const clone = (value) => JSON.parse(JSON.stringify(value));
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    const sameProject = (a, b) => a == null || b == null ? a === b
      : a.name === b.name && same(a.rootPaths, b.rootPaths);
    const own = (value, name) => Object.prototype.hasOwnProperty.call(value, name);
    const id = (value) => typeof value === "string" && value.length > 0 && value.length <= 200
      && !["__proto__", "constructor", "prototype"].includes(value) && !value.startsWith("linked:");
    const normalizeHome = (value) => {
      if (typeof value !== "string" || !value) throw Error("Local project home unavailable");
      const path = value.replace(/\\/g, "/").replace(/^\/\/\?\//, "").replace(/\/+$/, "");
      return /^[a-z]:|^\/\//i.test(path) ? path.toLowerCase() : path;
    };
    const project = (value, projectId) => {
      if (!plain(value) || value.id !== projectId || !id(projectId)
          || typeof value.name !== "string" || !value.name.trim()
          || !Array.isArray(value.rootPaths) || !value.rootPaths.length
          || value.rootPaths.some((path) => typeof path !== "string"
            || !/^(?:[a-z]:[\\/]|\/|\\\\)/i.test(path) || /:\/\//.test(path))
          || !Number.isFinite(value.createdAt) || !Number.isFinite(value.updatedAt)
          || Object.keys(value).some((name) => /chatgpt|backing|remote|durable|origin/i.test(name)
            && value[name] != null)) return null;
      return { id: projectId, name: value.name, rootPaths: [...value.rootPaths],
        createdAt: value.createdAt, updatedAt: value.updatedAt };
    };
    const projects = (value) => Object.fromEntries(Object.entries(plain(value) ? value : {})
      .flatMap(([name, record]) => {
        const safe = project(record, name);
        return safe ? [[name, safe]] : [];
      }));
    const state = services.applicationState;
    const nativeProjects = services.projects;
    const nativeMemberships = services.threadProjectAssignments;
    const read = async (name) => {
      const result = await state.read(name);
      if (!plain(result) || !own(result, "value")) throw Error("Unsupported local project state API");
      return result.value;
    };
    const write = async (name, value) => {
      const result = await state.write(name, value);
      if (result?.success !== true) throw Error("Local project state write was refused");
    };
    let home;
    let stopped = false;
    let epoch = 0;
    let nativeEpoch = 0;
    let busy = false;
    let pending = false;
    let flight = null;
    let stable = null;
    let restorePending = false;
    let restartBudget = 3;
    let restoreSeen = null;
    let notificationTimer = null;
    const notifications = [];
    const expectedMemberships = [];
    const active = () => !stopped && enabled();
    const owner = async () => {
      const value = await read(ownerKey);
      if (value == null) return null;
      if (!plain(value) || !(value.owner === null || typeof value.owner === "string")
          || !plain(value.inactive)) throw Error("Unsupported account project state");
      return value.owner;
    };
    const check = async (token) => {
      const current = await owner();
      if (!active() || token.epoch !== epoch || token.owner !== current) {
        const error = Error("Local project identity changed");
        error.name = "CodexLocalProjectScopeChanged";
        throw error;
      }
    };
    const native = async (token, operation) => {
      await check(token);
      const result = await operation();
      await check(token);
      return result;
    };
    const writeMemberships = async (token, records) => {
      expectedMemberships.push(...records.map(clone));
      return native(token, () => nativeMemberships.restoreMemberships(records));
    };
    const readView = async (token) => native(token, async () => {
      const [rawProjects, memberships, order, hosts] = await Promise.all([
        nativeProjects.getLocalProjects(), nativeMemberships.readMemberships(),
        read("project-order"), read("thread-project-membership-host-ids"),
      ]);
      if (!plain(rawProjects) || !plain(memberships?.assignments)
          || !Array.isArray(memberships.projectlessThreadIds)
          || (order != null && !Array.isArray(order))
          || (hosts != null && !plain(hosts))) throw Error("Unsupported local project metadata");
      return { rawProjects, projects: projects(rawProjects), memberships, order: order || [], hosts: hosts || {} };
    });
    const membership = (view, threadId, allowed, previouslyLocal = false) => {
      if (!id(threadId) || (view.hosts[threadId] != null && view.hosts[threadId] !== "local")) return null;
      const assignment = view.memberships.assignments[threadId];
      if (assignment != null) {
        if (assignment.projectKind !== "local" || assignment.projectOrigin != null
            || !own(allowed, assignment.projectId)) return null;
        return { threadId, hostId: "local",
          assignment: { projectKind: "local", projectId: assignment.projectId }, projectless: false };
      }
      if (!previouslyLocal) return null;
      return { threadId, hostId: "local", assignment: null,
        projectless: view.memberships.projectlessThreadIds.includes(threadId) };
    };
    const load = async (token) => {
      const value = await native(token, () => read(key));
      if (value == null) return null;
      if (!plain(value) || value.version !== 1 || value.home !== home
          || !Number.isSafeInteger(value.revision) || value.revision < 1
          || !plain(value.projects) || !plain(value.memberships)
          || !Array.isArray(value.knownProjects) || !Array.isArray(value.order)
          || value.knownProjects.some((value) => !id(value))
          || !same(projects(value.projects), value.projects)
          || Object.entries(value.memberships).some(([threadId, entry]) => !id(threadId)
            || entry?.threadId !== threadId || entry.hostId !== "local"
            || typeof entry.projectless !== "boolean"
            || (entry.assignment !== null && (entry.assignment?.projectKind !== "local"
              || !own(value.projects, entry.assignment.projectId)))))
        throw Error("Unsupported shared local project record");
      return value;
    };
    const save = async (token, value) => {
      const next = { ...value, revision: value.revision + 1 };
      if (JSON.stringify(next).length > 4 * 1024 * 1024) throw Error("Local project record is too large");
      await native(token, () => write(key, next));
      return next;
    };
    const bootstrap = async (token, view) => {
      const container = await native(token, () => read(ownerKey));
      const sources = [];
      if (container != null) {
        if (!plain(container.inactive)) throw Error("Unsupported inactive local project state");
        for (const cached of Object.values(container.inactive)) {
          if (!plain(cached?.globals)) throw Error("Unsupported inactive local project state");
          const globals = cached.globals;
          sources.push({ projects: projects(globals[projectKey]), order: globals["project-order"] || [],
            hosts: globals["thread-project-membership-host-ids"] || {},
            memberships: { assignments: globals[membershipKey] || {},
              projectlessThreadIds: globals["projectless-thread-ids"] || [] } });
        }
      }
      sources.push(view);
      const seed = { version: 1, home, revision: 0, projects: {}, memberships: {}, knownProjects: [], order: [] };
      for (const source of sources) {
        for (const [name, value] of Object.entries(source.projects)) {
          if (source === view || !own(seed.projects, name) || value.updatedAt >= seed.projects[name].updatedAt)
            seed.projects[name] = value;
        }
        if (!Array.isArray(source.order) || !plain(source.hosts)
            || !plain(source.memberships.assignments) || !Array.isArray(source.memberships.projectlessThreadIds))
          throw Error("Unsupported inactive local project metadata");
        for (const threadId of Object.keys(source.memberships.assignments)) {
          const entry = membership(source, threadId, source.projects);
          if (entry) seed.memberships[threadId] = entry;
        }
        seed.order = [...new Set([...source.order.filter((name) => own(seed.projects, name)), ...seed.order])];
      }
      // Never turn an active cloud-associated record into an ordinary local project.
      for (const name of Object.keys(view.rawProjects)) {
        if (own(seed.projects, name) && !own(view.projects, name)) {
          delete seed.projects[name];
          for (const [threadId, entry] of Object.entries(seed.memberships))
            if (entry.assignment?.projectId === name) delete seed.memberships[threadId];
        }
      }
      seed.knownProjects = Object.keys(seed.projects);
      seed.order = seed.order.filter((name) => own(seed.projects, name));
      seed.backup = clone({ projects: seed.projects, memberships: seed.memberships, order: seed.order });
      return save(token, seed);
    };
    const restore = async (token, shared, view) => {
      let next = { ...view.rawProjects };
      const conflicted = new Set();
      restoreSeen = new Set(Object.keys(view.projects));
      const willUpsert = new Set(shared.knownProjects.filter((name) => own(shared.projects, name)
        && !(own(next, name) && !own(view.projects, name))
        && !sameProject(view.projects[name], shared.projects[name])));
      // Native upsert fixes cwd-based classifications before adding roots. Temporarily
      // mark our known unassigned threads so it cannot infer a different membership.
      const protectedThreads = Object.values(shared.memberships).filter((entry) =>
        (entry.assignment ? willUpsert.has(entry.assignment.projectId) : willUpsert.size > 0)
        && !view.memberships.assignments[entry.threadId]
        && !view.memberships.projectlessThreadIds.includes(entry.threadId)
        && (view.hosts[entry.threadId] == null || view.hosts[entry.threadId] === "local"))
        .map((entry) => ({ threadId: entry.threadId, hostId: "local", assignment: null, projectless: true }));
      let completed = false;
      try {
        if (protectedThreads.length) await writeMemberships(token, protectedThreads);
        for (const name of shared.knownProjects) {
          if (own(next, name) && !own(view.projects, name)) { conflicted.add(name); continue; }
          if (own(shared.projects, name)) {
            const value = shared.projects[name];
            if (willUpsert.has(name)) {
              // Establish the native legacy/server ID mapping, not just the UI cache.
              restoreSeen.add(name);
              const restored = await native(token, () => nativeProjects.upsertLocal({
                projectId: name, name: value.name, sources: value.rootPaths,
              }, "restore"));
              if (restored !== true) throw Error("Native local project restoration was refused");
            }
          } else if (own(view.projects, name)) {
            await native(token, () => nativeProjects.removeLocal(name));
          }
        }
        view = await readView(token);
        next = view.rawProjects;
        const order = [...new Set([...shared.order.filter((name) => own(next, name) && !conflicted.has(name)),
          ...view.order.filter((name) => !shared.knownProjects.includes(name) || own(shared.projects, name))])];
        if (!same(order, view.order))
          await native(token, () => nativeProjects.setProjectOrder(order));
        const changed = Object.values(shared.memberships).filter((entry) => {
          if (view.hosts[entry.threadId] != null && view.hosts[entry.threadId] !== "local") return false;
          const existing = view.memberships.assignments[entry.threadId];
          if (existing && (existing.projectKind !== "local" || existing.projectOrigin != null
              || !own(view.projects, existing.projectId))) return false;
          if (entry.assignment && !project(next[entry.assignment.projectId], entry.assignment.projectId)) return false;
          const current = view.memberships.assignments[entry.threadId] || null;
          return !same(current, entry.assignment)
            || view.memberships.projectlessThreadIds.includes(entry.threadId) !== entry.projectless;
        });
        if (changed.length) await writeMemberships(token, changed);
        const verified = await readView(token);
        for (const name of shared.knownProjects) {
          if (own(verified.rawProjects, name) && !own(verified.projects, name)) continue;
          if (!sameProject(verified.projects[name], shared.projects[name]))
            throw Error("Native local project restoration was not retained");
        }
        for (const entry of changed) {
          if (!same(verified.memberships.assignments[entry.threadId] || null, entry.assignment)
              || verified.memberships.projectlessThreadIds.includes(entry.threadId) !== entry.projectless)
            throw Error("Native local project membership restoration was not retained");
        }
        completed = true;
        return verified;
      } finally {
        // Undo only our temporary markers, never a newer assignment or another scope.
        if (!completed && protectedThreads.length && token.nativeEpoch === nativeEpoch && token.owner === await owner()) {
          const current = await nativeMemberships.readMemberships();
          const rollback = protectedThreads.filter((entry) => !current.assignments[entry.threadId]
            && current.projectlessThreadIds.includes(entry.threadId))
            .map((entry) => ({ ...entry, projectless: false }));
          if (rollback.length && token.nativeEpoch === nativeEpoch && token.owner === await owner()) {
            expectedMemberships.push(...rollback.map(clone));
            await nativeMemberships.restoreMemberships(rollback);
          }
        }
      }
    };
    const locked = async (operation) => {
      if (!active()) return;
      if (!locks || typeof locks.request !== "function") throw Error("Cross-window local project lock unavailable");
      home ||= normalizeHome(await services.executionPaths.readCodexHome("local"));
      return locks.request(`codex-plus-local-projects:${home}`, async () => {
        if (!active()) return;
        if (normalizeHome(await services.executionPaths.readCodexHome("local")) !== home)
          throw Error("Local project home changed");
        const token = { epoch, nativeEpoch, owner: await owner() };
        busy = true;
        try {
          let view = await readView(token);
          let shared = await load(token);
          shared ||= await bootstrap(token, view);
          const identityChanged = !stable || stable.owner !== token.owner || stable.epoch !== token.epoch;
          const events = notifications.splice(0);
          if (!identityChanged && events.length) {
            const next = clone(shared);
            const projectEvent = events.some((event) => event.type === "global-state-updated"
              && event.keys?.includes(projectKey));
            const deletionEvent = events.some((event) => event.type === "global-state-updated"
              && [projectKey, "project-order", "pinned-project-ids"].every((name) => event.keys?.includes(name)));
            if (projectEvent && (Object.keys(view.projects).length || !Object.keys(shared.projects).length || deletionEvent)) {
              Object.assign(next.projects, view.projects);
              for (const name of Object.keys(view.projects))
                if (!next.knownProjects.includes(name)) next.knownProjects.push(name);
              if (deletionEvent)
                for (const name of shared.knownProjects)
                  if (!own(view.rawProjects, name) && events.some((event) =>
                    event.keys?.includes("pinned-project-ids") && (!event.seen || event.seen.includes(name))))
                    delete next.projects[name];
              next.order = view.order.filter((name) => own(next.projects, name));
            }
            for (const name of Object.keys(view.rawProjects)) {
              if (own(view.projects, name) || !next.knownProjects.includes(name)) continue;
              delete next.projects[name];
              next.knownProjects = next.knownProjects.filter((value) => value !== name);
              for (const [threadId, entry] of Object.entries(next.memberships))
                if (entry.assignment?.projectId === name) delete next.memberships[threadId];
            }
            for (const event of events) {
              if (event.type !== "thread-project-memberships-updated" || !Array.isArray(event.memberships)) continue;
              for (const entry of event.memberships) {
                if (!id(entry?.threadId)) continue;
                if (event.duringRestore && event.epoch === token.epoch
                    && typeof entry.projectless === "boolean"
                    && (entry.hostId == null || entry.hostId === "local")
                    && (entry.assignment == null || (entry.assignment.projectKind === "local"
                      && entry.assignment.projectOrigin == null
                      && own(view.projects, entry.assignment.projectId)))
                    && (own(shared.memberships, entry.threadId) || entry.assignment != null)
                    && (view.hosts[entry.threadId] == null || view.hosts[entry.threadId] === "local")) {
                  const record = { threadId: entry.threadId, hostId: "local",
                    assignment: entry.assignment == null ? null
                      : { projectKind: "local", projectId: entry.assignment.projectId },
                    projectless: entry.projectless };
                  await writeMemberships(token, [record]);
                  if (record.assignment) {
                    const name = record.assignment.projectId;
                    next.projects[name] = view.projects[name];
                    if (!next.knownProjects.includes(name)) next.knownProjects.push(name);
                  }
                  next.memberships[entry.threadId] = record;
                  continue;
                }
                const current = membership(view, entry.threadId, view.projects, own(shared.memberships, entry.threadId));
                // Delayed notifications cannot overwrite a newer native decision.
                if (current && same(current.assignment, entry.assignment ?? null)
                    && current.projectless === entry.projectless) {
                  if (current.assignment && !own(next.projects, current.assignment.projectId)) {
                    const name = current.assignment.projectId;
                    next.projects[name] = view.projects[name];
                    if (!next.knownProjects.includes(name)) next.knownProjects.push(name);
                    next.order = view.order.filter((name) => own(next.projects, name));
                  }
                  next.memberships[entry.threadId] = current;
                } else if ((view.hosts[entry.threadId] != null && view.hosts[entry.threadId] !== "local")
                    || (view.memberships.assignments[entry.threadId] != null && !current))
                  delete next.memberships[entry.threadId];
              }
            }
            for (const [threadId, entry] of Object.entries(next.memberships))
              if (entry.assignment && !own(next.projects, entry.assignment.projectId))
                next.memberships[threadId] = { threadId, hostId: "local", assignment: null, projectless: true };
            if (!same(shared, next)) shared = await save(token, next);
            view = await readView(token);
          }
          stable = { owner: token.owner, epoch: token.epoch };
          if (identityChanged || restorePending) {
            restorePending = true;
            view = await restore(token, shared, view);
            restorePending = false;
          }
          onReady({ status: "ready", revision: shared.revision,
            projects: Object.keys(shared.projects).length, memberships: Object.keys(shared.memberships).length });
          return await operation({ token, shared, view });
        } finally {
          restoreSeen = null;
          expectedMemberships.length = 0;
          busy = false;
          if (notifications.length) pending = true;
          if (pending && !flight && active()) queueMicrotask(() => void reconcile());
        }
      });
    };
    const reconcile = () => {
      if (!active()) return Promise.resolve();
      pending = true;
      if (flight) return flight;
      flight = (async () => {
        let attempts = 0;
        while (pending && active() && attempts++ < 3) {
          pending = false;
          await locked(async () => {});
        }
      })().catch((error) => {
        if (error?.name !== "CodexLocalProjectScopeChanged")
          onError("Local project continuity is unavailable; native state was left in control.", error);
      })
        .finally(() => {
          flight = null;
          if (pending && active() && restartBudget-- > 0) queueMicrotask(() => void reconcile());
          else if (!pending) restartBudget = 3;
        });
      return flight;
    };
    const capture = async ({ token, shared, view: before }, operation, canRemove) => {
      const after = await readView(token);
      const next = clone(shared);
      const abandoned = new Set();
      for (const [name, value] of Object.entries(after.projects)) {
        if (!same(before.projects[name], value)) {
          next.projects[name] = value;
          if (!next.knownProjects.includes(name)) next.knownProjects.push(name);
        }
      }
      for (const name of Object.keys(before.projects)) {
        if (own(after.rawProjects, name) && !own(after.projects, name)) {
          delete next.projects[name];
          next.knownProjects = next.knownProjects.filter((value) => value !== name);
          abandoned.add(name);
        } else if (canRemove && !own(after.rawProjects, name)) delete next.projects[name];
      }
      const threads = new Set([...Object.keys(before.memberships.assignments),
        ...Object.keys(after.memberships.assignments), ...Object.keys(shared.memberships)]);
      for (const threadId of threads) {
        const a = membership(before, threadId, before.projects, own(shared.memberships, threadId));
        const b = membership(after, threadId, after.projects, !!a || own(shared.memberships, threadId));
        if (!same(a, b)) {
          if (b) next.memberships[threadId] = b;
          else delete next.memberships[threadId];
        }
      }
      for (const [threadId, entry] of Object.entries(next.memberships)) {
        if (entry.assignment && abandoned.has(entry.assignment.projectId)) delete next.memberships[threadId];
        else if (entry.assignment && !own(next.projects, entry.assignment.projectId))
          next.memberships[threadId] = { threadId, hostId: "local", assignment: null, projectless: true };
      }
      if (!same(before.order, after.order)) next.order = after.order.filter((name) => own(next.projects, name));
      if (!same(next, shared)) await save(token, next);
      return operation;
    };
    const mutations = {
      projects: new Set(["createOrSelectLocalProjects", "createProjectsForRoots", "createProjectForRoot",
        "createLocal", "editLocal", "renameLocal", "upsertLocal", "removeLocal", "setProjectOrder"]),
      threadProjectAssignments: new Set(["setAssignment", "setProjectless", "setMembership",
        "restoreMemberships", "assignIfUnassigned", "moveLocalProjectThreadsToProjectless", "reset",
        "updateProjectlessThread"]),
      applicationState: new Set(["write"]),
    };
    const originalServices = {};
    const wrappers = {};
    for (const [name, methods] of Object.entries(mutations)) {
      const target = services[name];
      originalServices[name] = target;
      const cache = new Map();
      wrappers[name] = new Proxy(target, {
        get(target, method) {
          const original = Reflect.get(target, method);
          if (!methods.has(method)) return original;
          if (!cache.has(method)) cache.set(method, async (...args) => {
            if (!active() || (name === "applicationState" && args[0] !== projectKey))
              return Reflect.apply(original, target, args);
            let invoked = false;
            let completed = false;
            let result;
            try {
              const outcome = await locked(async (context) => {
                result = await native(context.token, async () => {
                  invoked = true;
                  const value = await Reflect.apply(original, target, args);
                  result = value;
                  completed = true;
                  return value;
                });
                return capture(context, result, method === "removeLocal" || name === "applicationState");
              });
              return invoked ? outcome : Reflect.apply(original, target, args);
            } catch (error) {
              if (error?.name !== "CodexLocalProjectScopeChanged")
                onError("Local project change could not be saved for other accounts.", error);
              // Never repeat a native command after it may already have run.
              if (completed) return result;
              if (invoked) throw error;
              return Reflect.apply(original, target, args);
            }
          });
          return cache.get(method);
        },
      });
      services[name] = wrappers[name];
      if (services[name] !== wrappers[name]) throw Error("Native project service cannot be wrapped");
    }
    return {
      key, reconcile,
      resume() { epoch++; stable = null; restorePending = true; restartBudget = 3; return reconcile(); },
      notify(message) {
        if (message?.type === "persisted-atom-sync") { epoch++; nativeEpoch++; }
        if (!active()) return;
        if (message?.type === "persisted-atom-sync") pending = true;
        const relevant = message?.type === "thread-project-memberships-updated"
          || message?.type === "persisted-atom-sync"
          || (message?.type === "global-state-updated"
            && message.keys?.some((name) => [projectKey, membershipKey, "project-order", ownerKey].includes(name)));
        if (relevant) {
          let event = clone(message);
          if (busy && message.type === "thread-project-memberships-updated" && Array.isArray(message.memberships)) {
            event.memberships = message.memberships.filter((record) => {
              const index = expectedMemberships.findIndex((entry) => entry.threadId === record?.threadId
                && same(entry.assignment, record.assignment ?? null) && entry.projectless === record.projectless);
              if (index < 0) return true;
              expectedMemberships.splice(index, 1);
              return false;
            });
          }
          if (message.type !== "persisted-atom-sync") notifications.push({
            ...event, epoch, duringRestore: restoreSeen !== null, seen: restoreSeen ? [...restoreSeen] : null,
          });
          if (!busy && notificationTimer === null) notificationTimer = setTimeout(() => {
            notificationTimer = null;
            void reconcile();
          }, 0);
        }
      },
      dispose() {
        stopped = true;
        epoch++;
        if (notificationTimer !== null) clearTimeout(notificationTimer);
        for (const name of Object.keys(wrappers))
          if (services[name] === wrappers[name]) services[name] = originalServices[name];
      },
    };
  }

  function installCodexLocalProjectContinuity() {
    window.__codexPlusLocalProjectContinuity?.dispose();
    let runtime = null;
    let starting = null;
    let disposed = false;
    let retry = null;
    let attempts = 0;
    const enabled = () => !disposed;
    const report = (message, error) => {
      const reason = error?.name === "CodexLocalProjectScopeChanged" ? "scope-changed" : "native-service-unavailable";
      window.__codexPlusLocalProjectContinuityStatus = { status: "unavailable", message, reason };
      if (typeof sendCodexPlusDiagnostic === "function")
        sendCodexPlusDiagnostic("local_project_continuity_unavailable", { reason });
    };
    const ready = (status) => {
      const wasReady = window.__codexPlusLocalProjectContinuityStatus?.status === "ready";
      window.__codexPlusLocalProjectContinuityStatus = status;
      if (!wasReady && typeof sendCodexPlusDiagnostic === "function")
        sendCodexPlusDiagnostic("local_project_continuity_ready", { projects: status.projects, memberships: status.memberships });
    };
    const sync = async () => {
      if (disposed) return;
      if (runtime) return runtime.reconcile();
      if (starting) return starting;
      starting = (async () => {
        const services = await loadCodexLocalProjectServices();
        if (disposed) return;
        runtime = createCodexLocalProjectContinuity({ services, locks: navigator.locks, enabled, onError: report, onReady: ready });
        await runtime.reconcile();
      })().catch((error) => report("Local project continuity is unavailable in this Codex version.", error))
        .finally(() => {
          starting = null;
          if (!disposed && !runtime && attempts < 8) {
            retry = setTimeout(() => { retry = null; void sync(); }, Math.min(5000, 250 * 2 ** attempts++));
          }
        });
      return starting;
    };
    const incoming = (event) => {
      if (event.source != null && event.source !== window) return;
      if (event.origin && event.origin !== "null" && event.origin !== window.location.origin) return;
      runtime?.notify(event.data);
    };
    window.addEventListener("message", incoming);
    const dispose = () => {
      disposed = true;
      if (retry !== null) clearTimeout(retry);
      window.removeEventListener("message", incoming);
      runtime?.dispose();
    };
    window.__codexPlusLocalProjectContinuity = { dispose };
    void sync();
  }
