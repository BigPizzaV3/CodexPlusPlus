const assert = require("node:assert/strict");
const http = require("node:http");
const path = require("node:path");

let server;
let gateway;
let qrSequence = 0;
const polls = [];
const waiting = [];

// 仅供 codex-isolated-testing 的 manager 场景使用。
function sendJson(response, status, body) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

async function until(predicate, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("Timed out waiting for the simulated gateway state");
}

function takeWaiting(token) {
  const index = waiting.findIndex((item) => item.token === token && !item.response.destroyed);
  assert.notEqual(index, -1, `No pending poll for ${token}`);
  return waiting.splice(index, 1)[0].response;
}

module.exports = {
  async prepare() {
    server = http.createServer((request, response) => {
      request.resume();
      const url = new URL(request.url, "http://127.0.0.1");
      if (url.pathname === "/ilink/bot/get_bot_qrcode") {
        qrSequence += 1;
        return sendJson(response, 200, {
          qrcode: `qr-${qrSequence}`,
          qrcode_img_content: `https://example.invalid/qr-${qrSequence}`,
        });
      }
      if (url.pathname === "/ilink/bot/get_qrcode_status") {
        const sequence = url.searchParams.get("qrcode").endsWith("-2") ? 2 : 1;
        return sendJson(response, 200, {
          status: "confirmed",
          bot_token: `fake-token-${sequence}`,
          ilink_bot_id: "fake-account",
          ilink_user_id: "fake-peer",
          baseurl: gateway,
        });
      }
      if (url.pathname === "/ilink/bot/getupdates") {
        const token = request.headers.authorization;
        polls.push(token);
        if (polls.filter((value) => value === token).length === 1) {
          return sendJson(response, 200, { ret: 0, msgs: [] });
        }
        waiting.push({ token, response });
        return;
      }
      sendJson(response, 404, { ret: 1 });
    });
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    });
    gateway = `http://127.0.0.1:${server.address().port}`;
  },

  async run({ managerPage, invoke, dialogs, outputDir, check }) {
    const statusIs = async (state) => (await invoke("weixin_connect_status")).state === state;
    const firstToken = "Bearer fake-token-1";
    const nextToken = "Bearer fake-token-2";

    const settings = await invoke("load_settings");
    settings.settings.weixinConnectBaseUrl = gateway;
    const saved = await invoke("save_settings", { settings: settings.settings });
    assert.equal(saved.status, "ok", saved.message);
    await managerPage.reload({ waitUntil: "domcontentloaded" });
    await dialogs.manager.ensureClear();
    const navigation = managerPage.locator('[data-agent-page="weixin"]');
    if (!(await navigation.isVisible())) {
      await managerPage.locator("button.cx-side-item").filter({ hasText: "Codex" }).first().click();
    }
    await navigation.click();
    await managerPage.getByRole("button", { name: "扫码登录", exact: true }).click();
    await until(() => statusIs("running"));
    await until(() => waiting.some((item) => item.token === firstToken && !item.response.destroyed));
    await managerPage.locator(".weixin-status-panel.is-running").waitFor();
    assert.equal((await invoke("load_settings")).settings.weixinConnectEnabled, true);
    check("QR confirmation through the manager UI enabled and started the simulated connection");
    await dialogs.manager.ensureClear();

    sendJson(takeWaiting(firstToken), 503, { ret: 1 });
    await until(() => statusIs("retrying"));
    await managerPage.locator(".weixin-status-panel.is-retrying").waitFor();
    await until(() => waiting.some((item) => item.token === firstToken && !item.response.destroyed));
    sendJson(takeWaiting(firstToken), 200, { ret: 0, msgs: [] });
    await until(() => statusIs("running"));
    await managerPage.locator(".weixin-status-panel.is-running").waitFor();
    check("manager UI recovered from a failed poll after an empty successful response");

    await until(() => waiting.some((item) => item.token === firstToken && !item.response.destroyed));
    const oldPoll = waiting.find((item) => item.token === firstToken && !item.response.destroyed);
    await managerPage.locator(".weixin-status-panel").getByRole("button", { name: "更多操作" }).click();
    await managerPage.getByRole("menuitem", { name: "重新登录" }).click();
    await until(() => polls.includes(nextToken));
    await until(() => oldPoll.response.destroyed);
    await until(() => statusIs("running"));
    const nextIndex = polls.indexOf(nextToken);
    assert(!polls.slice(nextIndex).includes(firstToken));
    await managerPage.locator(".weixin-status-panel.is-running").waitFor();
    await dialogs.manager.ensureClear();
    await managerPage.screenshot({ path: path.join(outputDir, "weixin-running.png") });
    check("re-authorization closed the old poll and used the new credential");

    const stopped = await invoke("weixin_connect_stop");
    assert.equal(stopped.status, "ok", stopped.message);
    await until(() => statusIs("stopped"));
    await managerPage.locator(".weixin-status-panel.is-stopped").waitFor();
    check("manual stop ended the isolated connection");
  },

  async cleanup() {
    if (!server) return;
    server.closeAllConnections();
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  },
};
