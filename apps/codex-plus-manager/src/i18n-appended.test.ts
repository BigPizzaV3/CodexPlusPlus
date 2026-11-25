import assert from "node:assert/strict";
import test from "node:test";
import * as EN from "./i18n-en.ts";
import { translatePlain } from "./i18n.ts";

const NOTICE = "现在的供应商要用真实登录，本地账号用不了，Codex 登录身份已退回「不登录」。";

test("后端接在消息后面的提示：英文界面下前后两半各自翻译", () => {
  assert.equal(translatePlain(EN, `已切换到纯 API。${NOTICE}`), "Switched to API-only mode. The current provider needs a real sign-in, so the local account can't be used; Codex sign-in identity is back to \"Not signed in\".");
  // 前半句是带变量的消息时也照常按模式翻译。
  assert.match(translatePlain(EN, `「示例」已从 1.0 更新到 1.1。${NOTICE}`), /^Updated "示例" from 1\.0 to 1\.1\. The current provider needs a real sign-in/);
  assert.match(translatePlain(EN, NOTICE), /^The current provider needs a real sign-in/);
});
