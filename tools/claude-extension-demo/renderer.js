// Claude 页面里的部分（清单的 claudeDesktop.renderer）：Chat、Cowork、Code 页面都会加载。
// 同一份代码也能在 Codex 里跑：codexPlus.host 区分宿主，codexPlus.supports() 问某个挂载点这里有没有。
codexPlus.extension((ctx) => {
  ctx.toast(`${ctx.config.greeting}（${codexPlus.host ?? "codex"}）`, { type: "success" });

  ctx.registerPage({
    title: "Claude 拓展连通测试",
    icon: "✦",
    render({ container }) {
      const root = document.createElement("div");
      root.className = "claude-extension-demo";
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = "调后台 status";
      const output = document.createElement("pre");
      output.textContent = "点上面的按钮：页面经 ctx.backend.call 调后台的 status。";
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const status = await ctx.backend.call("status", { source: "page" });
          output.textContent = JSON.stringify(status, null, 2);
        } catch (error) {
          output.textContent = `调用失败：${error.message}`;
        } finally {
          button.disabled = false;
        }
      });
      root.append(button, output);
      container.append(root);
    },
  }, { id: "main" });

  // 输入框工具行里的按钮（挨着加号）：把当前时间写进输入框。
  ctx.registerComposerAction({
    label: "插入当前时间",
    icon: "◷",
    onActivate: ({ insertText }) => insertText(new Date().toLocaleTimeString()),
  });

  // 每条消息操作行里的按钮：报一下这条消息有多少字。
  ctx.registerMessageAction({
    label: "字数",
    icon: "#",
    onActivate: ({ message }) => ctx.toast(`${message.role === "user" ? "你" : "Claude"} 的这条消息有 ${message.text.length} 个字`),
  });

  // 斜杠命令：在输入框里打「/demo 任意文字」回车，交给拓展处理，不会发给 Claude。
  ctx.registerCommand({
    name: "demo",
    description: "连通测试的斜杠命令",
    run: (args, { toast }) => toast(`收到命令参数：${args || "（空）"}`),
  });

  ctx.registerShortcut({
    key: "Mod+Shift+E",
    description: "显示连通测试的提示",
    run: () => ctx.toast("快捷键生效"),
  });
});
