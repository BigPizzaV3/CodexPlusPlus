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
  const install = () => {
    if (document.querySelector(`[${marker}]`)) return;
    const node = composer();
    if (!node?.parentElement) return;
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute(marker, "true");
    button.title = "Polish draft";
    button.textContent = "✨";
    button.addEventListener("click", async () => {
      const text = read(node).trim();
      if (!text) return;
      button.disabled = true;
      try {
        const headers = { "content-type": "application/json" };
        if (bridgeToken) headers["x-xuan-bridge-token"] = bridgeToken;
        const response = await fetch(`${bridgeUrl}/v1/polish`, {
          method: "POST", headers,
          body: JSON.stringify({ text, style: "structured" })
        });
        const payload = await response.json();
        if (payload?.text) write(node, payload.text);
      } finally { button.disabled = false; }
    });
    node.parentElement.append(button);
  };
  new MutationObserver(install).observe(document.documentElement, { childList: true, subtree: true });
  install();
})();
