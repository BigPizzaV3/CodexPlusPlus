// 注入页打包供应商编辑函数时替换 @/i18n：注入页只有中文界面，原文直出，`{0}` 占位按参数替换。
export function t(zh) {
  return zh;
}

export function tf(key, args) {
  return String(key).replace(/\{(\d+)\}/g, (match, index) => {
    const value = args[Number(index)];
    return value === undefined || value === null ? match : String(value);
  });
}
