// 脚本市场与本地脚本列表的纯逻辑（#2781）。不依赖 i18n / React，便于 node --test 直接测；
// 文案由调用方按返回的种类翻译。

/** 市场排序：清单原顺序（维护者编排）、最近更新优先、按名称。 */
export type ScriptMarketSort = "default" | "updated" | "name";

type SortableMarketScript = { name: string; updatedAt?: string };

// `YYYY-MM-DD HH:mm[:ss[.sss]]`，后面可跟时区（Z、+08:00、UTC 等）。
const SPACED_DATE_TIME = /^(\d{4}-\d{2}-\d{2})\s+(\d{1,2})(:\d{2}(?::\d{2}(?:\.\d+)?)?)\s*(.*)$/;
// 早于 1973 年的数字不当成 Unix 时间（避免把「2026」这种年份当成秒数）。
const MIN_UNIX_SECONDS = 1e8;
// 小于这个值按秒，否则按毫秒（1e11 秒已是公元 5000 年后）。
const UNIX_MS_THRESHOLD = 1e11;

function unixTime(value: number): number | null {
  if (!Number.isFinite(value) || value < MIN_UNIX_SECONDS) return null;
  return value < UNIX_MS_THRESHOLD ? Math.round(value * 1000) : Math.round(value);
}

/**
 * 解析清单里的更新时间；空串、非法值返回 null（视为「未标注」）。
 *
 * 不能直接 `new Date(text)`：「2026-10-29 12:00」这种带空格的写法 Windows WebView2 能解析，
 * macOS WKWebView 却得到 Invalid Date。这里支持 ISO 8601、`YYYY-MM-DD`、
 * `YYYY-MM-DD HH:mm[:ss]`（空格换成 T 再解析，不带时区按本地时间）和 Unix 秒 / 毫秒数字。
 */
export function parseMarketTime(value: string | number | undefined | null): number | null {
  if (typeof value === "number") return unixTime(value);
  const text = (value ?? "").trim();
  if (!text) return null;
  // 纯数字：Unix 秒 / 毫秒；只有 4 位年份（ISO 8601 的 `YYYY`）交给下面按日期解析，其余当没有时间。
  if (/^\d+(\.\d+)?$/.test(text) && !/^\d{4}$/.test(text)) return unixTime(Number(text));
  const spaced = SPACED_DATE_TIME.exec(text);
  let normalized = text;
  if (spaced) {
    const [, date, hour, rest, zone] = spaced;
    const offset = /^(utc|gmt)$/i.test(zone) ? "Z" : zone;
    normalized = `${date}T${hour.padStart(2, "0")}${rest}${offset}`;
  }
  const time = Date.parse(normalized);
  return Number.isNaN(time) ? null : time;
}

/**
 * 默认排序：清单里每个脚本都标了有效更新时间时按「最近更新」排，
 * 否则保持清单原顺序——只有部分脚本有时间时，按时间排会把没标的老脚本和
 * 真正停更的脚本混在末尾，反而误导。
 */
export function defaultMarketSort(scripts: SortableMarketScript[]): ScriptMarketSort {
  if (!scripts.length) return "default";
  return scripts.every((script) => parseMarketTime(script.updatedAt) !== null) ? "updated" : "default";
}

/** 返回排序后的新数组，不改入参。同值保持清单原顺序；没有更新时间的排在最后。 */
export function sortMarketScripts<T extends SortableMarketScript>(scripts: T[], sort: ScriptMarketSort): T[] {
  if (sort === "default") return scripts.slice();
  const indexed = scripts.map((script, index) => ({ script, index, time: parseMarketTime(script.updatedAt) }));
  indexed.sort((left, right) => {
    if (sort === "updated") {
      if (left.time !== right.time) {
        if (left.time === null) return 1;
        if (right.time === null) return -1;
        return right.time - left.time;
      }
    } else {
      const byName = left.script.name.localeCompare(right.script.name, undefined, { sensitivity: "base", numeric: true });
      if (byName) return byName;
    }
    return left.index - right.index;
  });
  return indexed.map((item) => item.script);
}

export type RelativeAge =
  | { unit: "justNow" }
  | { unit: "hours" | "days" | "months" | "years"; value: number };

/** 距今多久。时钟偏差导致的「未来时间」按刚刚处理。 */
export function relativeAge(time: number, now: number): RelativeAge {
  const hour = 60 * 60 * 1000;
  const day = 24 * hour;
  const elapsed = Math.max(0, now - time);
  if (elapsed < hour) return { unit: "justNow" };
  if (elapsed < day) return { unit: "hours", value: Math.floor(elapsed / hour) };
  if (elapsed < 30 * day) return { unit: "days", value: Math.floor(elapsed / day) };
  if (elapsed < 365 * day) return { unit: "months", value: Math.floor(elapsed / (30 * day)) };
  return { unit: "years", value: Math.floor(elapsed / (365 * day)) };
}

/**
 * 本地脚本合并后的单列状态。
 *
 * 原来「启用 / 关闭」（用户开关）和 status（loaded / disabled …）分两列：关闭时两列都在说
 * 「关了」，开启时第二列才有信息。合并后开关单独成控件，状态只描述开关之外的事实：
 * 整体开关关着、已加载、加载中、加载失败、未加载（Codex 未运行或还没热重载）。
 */
export type LocalScriptState = "off" | "globalOff" | "loaded" | "loading" | "failed" | "notLoaded";

export function localScriptState(script: { enabled: boolean; status: string }, globalEnabled: boolean): LocalScriptState {
  if (!script.enabled) return "off";
  if (!globalEnabled) return "globalOff";
  switch (script.status) {
    case "loaded":
      return "loaded";
    case "loading":
      return "loading";
    case "failed":
      return "failed";
    default:
      return "notLoaded";
  }
}
