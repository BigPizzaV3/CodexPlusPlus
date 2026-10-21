const assert = require("node:assert/strict");
const path = require("node:path");

module.exports = {
  async run({ managerPage: page, dialogs, outputDir, check }) {
    await page.evaluate(() => {
      const skills = Array.from({ length: 24 }, (_, index) => ({
        id: `skill-${index}`, name: `skill-${index}`, description: "View state fixture",
        repoKey: `acme/repo-${String(index).padStart(2, "0")}@main`,
        installed: true, enabled: true, bundled: false, updateAvailable: false,
      }));
      for (let index = 0; index < 70; index++) skills.push({
        ...skills[0], id: `large-${index}`, name: `large-${index}`,
      });
      skills.push({ ...skills[0], id: "discover", name: "discover", repoKey: "acme/market@main", installed: false });
      const nativeFetch = window.fetch;
      window.fetch = async function(resource, options) {
        const command = String(resource).match(/ipc\.localhost\/([^/?]+)/)?.[1];
        if (!["list_installed_skills", "refresh_skill_catalog"].includes(command)) return nativeFetch.call(this, resource, options);
        return new Response(JSON.stringify({
          status: "ok", message: "", skills, repos: [], repositorySkills: [],
          repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "",
        }), { headers: { "Content-Type": "application/json", "Tauri-Response": "ok" } });
      };
    });
    const enter = async () => {
      await page.getByRole("button", { name: "Skills", exact: true }).click();
      await page.locator(".mg-skill-group").first().waitFor();
      await page.getByRole("button", { name: "刷新目录", exact: true }).waitFor();
      await dialogs.manager.ensureClear();
    };
    const returnToSkills = async () => {
      await page.getByRole("button", { name: "概览", exact: true }).click();
      await enter();
    };
    const group = page.locator('.mg-skill-group[data-group="acme/repo-00"]');
    const search = page.getByRole("textbox", { name: "搜索 Skills 或仓库" });
    await enter();
    assert.equal(await page.getByRole("button", { name: /^已安装/ }).getAttribute("data-active"), "true");
    assert.equal(await page.locator(".mg-skill-list").count(), 0);
    assert.equal(await page.locator('.mg-group-head[aria-expanded="false"]').count(), 24);
    check("first entry defaults to installed with every repository collapsed");
    await group.locator(".mg-group-head").click();
    const listScroll = await group.locator(".mg-skill-list").evaluate(node => {
      node.scrollTop = 680;
      return node.scrollTop;
    });
    await returnToSkills();
    assert.equal(await group.locator(".mg-group-head").getAttribute("aria-expanded"), "true");
    assert.equal(await group.locator(".mg-skill-list").evaluate(node => node.scrollTop), listScroll);
    assert.equal(await page.locator(".mg-skill-list").count(), 1);
    await page.locator(".mg-content").evaluate(node => { node.scrollTop = 360; });
    await page.waitForFunction(() => document.querySelector(".mg-content").scrollTop === 360);
    await returnToSkills();
    assert.equal(await page.locator(".mg-content").evaluate(node => node.scrollTop), 360);
    check("expanded repositories and page scroll survive feature navigation; actual group-list scroll position retained");
    await search.fill("skill-12");
    await page.locator(".mg-skill-name strong").filter({ hasText: /^skill-12$/ }).waitFor();
    await returnToSkills();
    assert.equal(await search.inputValue(), "skill-12");
    assert.equal(await page.locator(".mg-skill-row").count(), 1);
    await search.fill("");
    await group.waitFor();
    await page.getByRole("button", { name: /^发现/ }).click();
    await page.locator('.mg-skill-group[data-group="acme/market"]').waitFor();
    await returnToSkills();
    assert.equal(await page.getByRole("button", { name: /^发现/ }).getAttribute("data-active"), "true");
    await page.getByRole("button", { name: /^已安装/ }).click();
    await page.getByRole("button", { name: "全部折叠", exact: true }).click();
    await returnToSkills();
    assert.equal(await page.locator(".mg-skill-list").count(), 0);
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await returnToSkills();
    assert.equal(await page.locator(".mg-skill-list").count(), 24);
    await dialogs.manager.ensureClear();
    await page.screenshot({ path: path.join(outputDir, "skills-view-restored.png"), fullPage: true });
    await dialogs.manager.ensureClear();
    check("search, selected tab, collapse-all and expand-all choices survive navigation using fixture IPC");
  },
};
