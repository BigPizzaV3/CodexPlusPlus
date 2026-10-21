const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

// 通过 codex-isolated-testing 的 manager 场景接口运行，仅对测试页面提供可控 IPC。
module.exports = {
  async run({ managerPage: page, dialogs, outputDir, check }) {
    await page.evaluate(() => {
      const skill = (id, repo, updateAvailable = true) => ({
        id, name: id, description: "后台检查测试", repoKey: `acme/${repo}@main`,
        installed: true, enabled: true, bundled: false, updateAvailable,
      });
      const skills = [skill("alpha", "one"), skill("omega", "two"), skill("late", "three", false)];
      const payload = () => JSON.parse(JSON.stringify({
        status: "ok", message: "", skills, repos: [], repositorySkills: [],
        repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "",
      }));
      window.__refreshTest = { refreshCount: 0, localCount: 0, calls: [], updates: {}, refreshes: {} };
      const state = window.__refreshTest;
      const nativeFetch = window.fetch;
      window.fetch = async function(resource, options) {
        const command = String(resource).match(/ipc\.localhost\/([^/?]+)/)?.[1];
        if (!["list_installed_skills", "refresh_skill_catalog", "update_skill"].includes(command)) {
          return nativeFetch.call(this, resource, options);
        }
        let result;
        if (command === "refresh_skill_catalog") {
          const index = ++state.refreshCount;
          const stale = payload();
          await new Promise(resolve => { state.refreshes[index] = resolve; });
          if (index === 1) {
            skills.find(item => item.id === "late").updateAvailable = true;
            stale.skills.find(item => item.id === "late").updateAvailable = true;
          } else if (index === 2) {
            const discovered = skill("newly-found", "four");
            skills.push(discovered);
            stale.skills.push(discovered);
          }
          result = stale;
        } else if (command === "update_skill") {
          const { id } = JSON.parse(options.body);
          state.calls.push(id);
          await new Promise(resolve => { state.updates[id] = resolve; });
          skills.find(item => item.id === id).updateAvailable = false;
          result = payload();
        } else {
          state.localCount++;
          result = payload();
        }
        return new Response(JSON.stringify(result), {
          headers: { "Content-Type": "application/json", "Tauri-Response": "ok" },
        });
      };
    });
    const group = name => page.locator(`.mg-skill-group[data-group="acme/${name}"]`);
    const batch = name => group(name).locator(".mg-skill-group-head").getByRole("button", { name: "更新全部", exact: true });
    const single = name => group(name).locator(".mg-skill-row").getByRole("button", { name: "更新", exact: true });
    await dialogs.manager.ensureClear();
    await page.getByRole("button", { name: "Skills", exact: true }).click();
    await page.getByRole("button", { name: /^已安装/ }).click();
    await page.waitForFunction(() => !!window.__refreshTest.refreshes[1]);
    await group("one").locator(".mg-group-head").click();
    assert.equal(await batch("one").isEnabled(), true);
    assert.equal(await batch("two").isEnabled(), true);
    assert.equal(await single("one").isEnabled(), true);
    assert.equal(await page.getByRole("button", { name: "正在检查…", exact: true }).isDisabled(), true);
    await single("one").click();
    await page.waitForFunction(() => !!window.__refreshTest.updates.alpha);
    await batch("two").click();
    assert.match(await group("two").locator(".mg-skill-update-progress").innerText(), /等待更新/);
    assert.deepEqual(await page.evaluate(() => window.__refreshTest.calls), ["alpha"]);
    await dialogs.manager.ensureClear();
    await page.screenshot({ path: path.join(outputDir, "updating-during-check.png"), fullPage: true });
    await dialogs.manager.ensureClear();
    check("cached individual and repository updates start or enqueue while remote catalog request is still pending");

    await page.evaluate(() => window.__refreshTest.refreshes[1]());
    await batch("three").waitFor();
    assert.equal(await batch("three").isEnabled(), true);
    assert.match(await page.locator(".mg-skill-update-summary").innerText(), /正在更新：alpha/);
    await page.evaluate(() => window.__refreshTest.updates.alpha());
    await page.waitForFunction(() => !!window.__refreshTest.updates.omega);
    await page.evaluate(() => window.__refreshTest.updates.omega());
    await page.getByText("更新完成：成功 2 个，失败 0 个", { exact: true }).waitFor();
    assert.equal(await batch("one").count(), 0);
    assert.equal(await batch("two").count(), 0);
    check("refresh completion during a running update preserves queue and discovers other available updates");

    await page.getByRole("button", { name: "刷新目录", exact: true }).click();
    await page.waitForFunction(() => !!window.__refreshTest.refreshes[2]);
    await batch("three").click();
    await page.waitForFunction(() => !!window.__refreshTest.updates.late);
    await page.evaluate(() => window.__refreshTest.updates.late());
    await page.getByText("更新完成：成功 3 个，失败 0 个", { exact: true }).waitFor();
    assert.equal(await batch("three").count(), 0);
    await page.evaluate(() => window.__refreshTest.refreshes[2]());
    await batch("four").waitFor();
    await page.getByRole("button", { name: "刷新目录", exact: true }).waitFor();
    assert.equal(await batch("three").count(), 0);
    assert.equal(await batch("four").isEnabled(), true);
    assert.equal(await page.evaluate(() => window.__refreshTest.localCount >= 2), true);
    assert.deepEqual(await page.evaluate(() => window.__refreshTest.calls), ["alpha", "omega", "late"]);
    await dialogs.manager.ensureClear();
    await page.screenshot({ path: path.join(outputDir, "check-after-update.png"), fullPage: true });
    await dialogs.manager.ensureClear();
    fs.writeFileSync(path.join(outputDir, "background-refresh-result.json"), JSON.stringify(
      await page.evaluate(() => ({
        refreshCount: window.__refreshTest.refreshCount,
        localCount: window.__refreshTest.localCount,
        calls: window.__refreshTest.calls,
      })), null, 2));
    check("stale refresh returning after update completion rereads current inventory, preserves success and includes newly discovered repository");
  },
};
