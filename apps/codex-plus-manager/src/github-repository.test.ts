import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { isGitHubRepositoryHomepage } from "./github-repository.ts";

describe("GitHub repository homepage detection", () => {
  it("accepts canonical GitHub repository URLs", () => {
    const urls = [
      "https://github.com/0xTotoroX/codex-hide-usage-alert",
      "https://www.github.com/0xTotoroX/tux-toolbar-buddy/",
      "https://github.com/0xTotoroX/codex-hide-usage-alert.git",
      "https://github.com/BigPizzaV3/CodexPlusPlus?tab=readme-ov-file#readme",
    ];

    for (const url of urls) assert.equal(isGitHubRepositoryHomepage(url), true, url);
  });

  it("rejects non-repository and deceptive URLs", () => {
    const urls = [
      "",
      "not a URL",
      "http://github.com/0xTotoroX/codex-hide-usage-alert",
      "https://github.com/0xTotoroX",
      "https://github.com/0xTotoroX/codex-hide-usage-alert/issues",
      "https://github.com.evil.example/0xTotoroX/codex-hide-usage-alert",
      "https://example.com/0xTotoroX/codex-hide-usage-alert",
    ];

    for (const url of urls) assert.equal(isGitHubRepositoryHomepage(url), false, url);
  });

  it("rejects reserved and malformed GitHub paths", () => {
    const urls = [
      "https://github.com/settings/profile",
      "https://github.com/marketplace/actions",
      "https://github.com/about/careers",
      "https://github.com/site/privacy",
      "https://github.com/0xTotoroX//codex-hide-usage-alert",
      "https://github.com/0xTotoroX/codex-hide-usage-alert%2Fissues",
      "https://github.com/-invalid/codex-hide-usage-alert",
    ];

    for (const url of urls) assert.equal(isGitHubRepositoryHomepage(url), false, url);
  });
});
