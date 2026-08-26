
  function stripOwnUi(clone) {
    clone.querySelectorAll?.(`[${ROOT_ATTR}], [${PAYLOAD_ATTR}]`).forEach((item) => item.remove());
    return clone;
  }

  function elementText(node) {
    if (!(node instanceof Element)) return normalizeText(node?.textContent || "");
    return normalizeText(stripOwnUi(node.cloneNode(true)).textContent || "");
  }

  function directText(node) {
    if (!(node instanceof Element)) return "";
    const clone = stripOwnUi(node.cloneNode(true));
    clone.querySelectorAll?.("button,[role='button'],svg").forEach((item) => item.remove());
    return normalizeText(clone.textContent || "");
  }
