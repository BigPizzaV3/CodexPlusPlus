function runNativeQuotaTests(createGate) {
  const equal = (actual, expected, message) => {
    if (!Object.is(actual, expected)) throw new Error(message);
  };
  const mixed = {
    relayProfilesEnabled: true, activeRelayId: "mix",
    relayProfiles: [{ id: "mix", relayMode: "official", officialMixApiKey: true }],
  };
  const official = { ...mixed, relayProfiles: [{ ...mixed.relayProfiles[0], officialMixApiKey: false }] };
  const raw = {
    plan_type: "plus", rate_limit: { allowed: false, limit_reached: true, used_percent: 100 },
    rate_limit_warning: { text: "keep" }, credits: { balance: 0 },
  };
  const publications = [];
  const listeners = [];
  class Query {
    constructor(queryKey, data) { this.queryKey = queryKey; this.state = { data }; }
    setData(data) {
      // 模拟 TanStack 结构共享：实际缓存对象不是传入对象。
      this.state.data = { ...data };
      publications.push(this.state.data);
      for (const listener of listeners) listener({ query: this });
      return this.state.data;
    }
  }
  const main = new Query(["rate-limit-status"], raw);
  const image = new Query(["rate-limit-status", "image-generation"], raw);
  const unrelated = new Query(["other"], raw);
  const cache = {
    getAll: () => [main, image, unrelated],
    findAll: () => [main, image],
    find: ({ queryKey }) => [main, image, unrelated].find(q => JSON.stringify(q.queryKey) === JSON.stringify(queryKey)),
    subscribe: listener => listeners.push(listener),
  };
  const client = {
    getQueryCache: () => cache,
    setQueryData(key, updater) {
      const query = cache.find({ queryKey: key });
      return query.setData(typeof updater === "function" ? updater(query.state.data) : updater);
    },
    invalidateQueries: () => Promise.resolve(),
  };
  const gate = createGate(client);
  const runtime = gate.installNative(mixed);
  equal(main.state.data.rate_limit.allowed, true, "existing exhausted cache must unlock");
  equal(main.state.data.rate_limit.limit_reached, false, "limit_reached must clear");
  equal(main.state.data.rate_limit.used_percent, 100, "usage percentage must survive");
  equal(main.state.data.rate_limit_warning, raw.rate_limit_warning, "alerts are outside standalone scope");
  equal(main.state.data.credits, raw.credits, "credits must survive");
  equal(image.state.data, raw, "image query must stay untouched");
  equal(unrelated.state.data, raw, "unrelated query must stay untouched");
  const listenerCount = listeners.length;
  equal(gate.installNative(mixed), runtime, "reinjection must reuse runtime");
  equal(listeners.length, listenerCount, "reinjection must not add subscriptions");

  const updated = { ...raw, rate_limit: { ...raw.rate_limit, used_percent: 92 } };
  main.setData(updated); // SSE / query.fetch 都走同一个发布点。
  equal(publications.at(-1).rate_limit.allowed, true, "subscribers must see unlocked data on first publication");
  runtime.update(official);
  equal(main.state.data.rate_limit.allowed, false, "switching back must restore official lock");
  equal(main.state.data.rate_limit.used_percent, 92, "restore latest server snapshot, not startup snapshot");

  runtime.update(mixed);
  let updaterAllowed;
  client.setQueryData(main.queryKey, previous => {
    updaterAllowed = previous.rate_limit.allowed;
    return { ...previous, account_id: "new" };
  });
  equal(updaterAllowed, false, "functional updater must receive real server data");
  equal(main.state.data.rate_limit.allowed, true, "functional update must still unlock");
  runtime.update(null);
  equal(main.state.data.rate_limit.allowed, false, "settings failure must restore lock");
  equal(main.state.data.account_id, "new", "functional update must survive restoration");

  runtime.update({ ...mixed, relayProfilesEnabled: false });
  main.setData(raw);
  equal(main.state.data.rate_limit.allowed, false, "disabled relay must not unlock new publication");
  runtime.update({ ...mixed, relayProfiles: [{ ...mixed.relayProfiles[0], relayMode: "pureApi" }] });
  equal(main.state.data.rate_limit.allowed, false, "pure API must not use official-mix bypass");
  runtime.update(mixed);
  main.setData({ usage: updated, envelope: "preserve" });
  equal(main.state.data.usage.rate_limit.allowed, true, "wrapped usage response must unlock");
  equal(main.state.data.envelope, "preserve", "response envelope must survive");
  runtime.update(official);
  equal(main.state.data.usage.rate_limit.allowed, false, "wrapped usage response must restore");
  return "Native quota policy, existing cache, publication, restoration, structural sharing and reinjection passed";
}
module.exports = runNativeQuotaTests;
if (require.main === module) {
  const fs = require("node:fs");
  const vm = require("node:vm");
  console.log(runNativeQuotaTests(client => {
    const context = {
      module: { exports: {} }, window: { __REACT_QUERY_CLIENT__: client },
      document: {}, setTimeout, clearTimeout,
    };
    vm.runInNewContext(fs.readFileSync(require.resolve("./api-quota-gate.js"), "utf8"), context);
    return context.module.exports;
  }));
}
