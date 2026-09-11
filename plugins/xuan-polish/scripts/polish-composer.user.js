(() => {
  const marker = "data-xuan-polish-button";
  const bridgeUrl = window.__XUAN_BRIDGE_URL__ || "http://127.0.0.1:57324";
  const bridgeToken = window.__XUAN_BRIDGE_TOKEN__ || "";
  const composer = () => document.querySelector("textarea, [contenteditable='true']");
  const read = (node) => node?.value ?? node?.textContent ?? "";
  const write = (node, value) => {
    if (!node) return;
    if ("value" in node) node.value = value;
    else node.textContent = value;
    node.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const bridgeRequest = async (path, payload) => {
    if (typeof window.__codexSessionDeleteBridge === "function") {
      return window.__codexSessionDeleteBridge(path, payload);
    }
    const headers = { "content-type": "application/json" };
    if (bridgeToken) headers["x-xuan-bridge-token"] = bridgeToken;
    const response = await fetch(`${bridgeUrl}${path}`, {
      method: "POST",
      headers,
      body: JSON.stringify(payload)
    });
    return response.json();
  };
  const install = () => {
    const permission = document.querySelector("button[aria-label='更改权限'], button[aria-label='Change permissions']");
    if (!composer() || !permission?.parentElement) return;
    let button = document.querySelector(`[${marker}]`);
    if (!button) {
      button = document.createElement("button");
      button.type = "button";
      button.setAttribute(marker, "true");
      button.textContent = "✨";
      button.addEventListener("click", async () => {
        const node = composer();
        const text = read(node).trim();
        if (!text) return;
        button.disabled = true;
        try {
          const payload = await bridgeRequest("/v1/polish", { text, style: "structured" });
          if (payload?.text) write(node, payload.text);
        } finally { button.disabled = false; }
      });
    }
    button.title = "润色输入内容";
    button.setAttribute("aria-label", "润色输入内容");
    button.className = permission.className;
    Object.assign(button.style, { width: "28px", minWidth: "28px", paddingInline: "0", justifyContent: "center" });
    if (permission.nextElementSibling !== button) permission.insertAdjacentElement("afterend", button);
  };
  new MutationObserver(install).observe(document.documentElement, { childList: true, subtree: true });
  install();
})();
