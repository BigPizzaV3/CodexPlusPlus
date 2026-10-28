/** 供应商名字 / 地址 → 打包的厂商图标（assets/providers，来源与许可见该目录的 LICENSE.txt）。纯函数，便于测试。 */

/** `-color` 结尾的是彩色图标，其余是单色图标（界面按当前文字颜色着色，深浅色都清楚）。 */
const BRAND_RULES: Array<[RegExp, string]> = [
  [/claude|anthropic/, "claude-color"],
  [/chatgpt|openai|\bgpt\b|codex/, "openai"],
  [/deepseek/, "deepseek-color"],
  [/z\.ai\b|\bzai\b/, "zai"],
  [/zhipu|bigmodel|\bglm\b|智谱/, "zhipu-color"],
  [/kimi|moonshot|月之暗面/, "kimi-color"],
  [/bailian|百炼/, "bailian-color"],
  [/qwen|dashscope|aliyun|通义|千问/, "qwen-color"],
  [/stepfun|阶跃/, "stepfun-color"],
  [/minimax|hailuo|海螺/, "minimax-color"],
  [/doubao|豆包/, "doubao-color"],
  [/volc|\bark\b|火山/, "volcengine-color"],
  [/qianfan|baidu|wenxin|千帆|百度|文心/, "baiducloud-color"],
  [/mimo|xiaomi|小米/, "xiaomimimo"],
  [/modelscope|魔搭/, "modelscope-color"],
  [/longcat/, "longcat-color"],
  [/openrouter/, "openrouter"],
  [/silicon|硅基/, "siliconcloud-color"],
  [/aihubmix/, "aihubmix-color"],
  [/novita/, "novita-color"],
  [/azure/, "azure-color"],
  [/grok/, "grok"],
  [/x\.ai\b|\bxai\b/, "xai"],
  [/ollama/, "ollama"],
  [/lm ?studio/, "lmstudio"],
  [/antigravity/, "antigravity-color"],
  [/vertex/, "vertexai-color"],
  [/gemini/, "gemini-color"],
  [/google/, "google-color"],
  [/copilot/, "githubcopilot"],
  [/mistral/, "mistral-color"],
  [/groq/, "groq"],
  [/together/, "together-color"],
  [/fireworks/, "fireworks-color"],
  [/hunyuan|混元/, "hunyuan-color"],
  [/tencent|腾讯/, "tencentcloud-color"],
  [/huawei|华为/, "huawei-color"],
  [/cursor/, "cursor"],
  [/devin/, "devin-color"],
  [/opencode/, "opencode"],
  [/perplexity/, "perplexity-color"],
  [/new-?api/, "newapi-color"],
  [/hugging ?face/, "huggingface-color"],
  [/nvidia/, "nvidia-color"],
];

/** 按名字、再按地址找厂商图标；找不到返回 null（界面用字母头像）。 */
export function brandIconKey(name: string, baseUrl = ""): string | null {
  for (const text of [name, baseUrl]) {
    const lower = text.trim().toLowerCase();
    if (!lower) continue;
    const hit = BRAND_RULES.find(([pattern]) => pattern.test(lower));
    if (hit) return hit[1];
  }
  return null;
}

export function brandIconIsColor(key: string): boolean {
  return key.endsWith("-color");
}

export const BRAND_ICON_KEYS = Array.from(new Set(BRAND_RULES.map(([, key]) => key)));
