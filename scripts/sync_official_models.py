r"""官方模型元数据同步：openai/codex 仓库 → assets/codex-models.json。

流程：拉取官方开源仓库的 codex-rs/models-manager/models.json →
与 assets/codex-models.json diff → 仅增量补入资产中不存在的 slug。

设计原则（与 AGENTS.md 一致）：
- 静态资产是「官方现役 + 存量模型」的超集，本脚本只增不删；
  官方下架的模型可能仍被第三方中转使用，删除属于人工决策。
- 同步的条目保持官方原值（含 use_responses_lite=true）；
  Codex++ 的产品级适配（如中转强制 lite=false）由 compat 合并层与
  builder 负责，不落在静态资产里。
- comp_hash 为服务端一致性标记，语义未公开，不带入静态资产。

用法：
    python scripts/sync_official_models.py            # 应用增量
    python scripts/sync_official_models.py --check    # 只报告差异，不写文件

退出码：0 = 无差异或已应用；1 = --check 模式下发现可补入的增量。
"""

import argparse
import json
import sys
import urllib.request
from pathlib import Path

SOURCE_URL = (
    "https://raw.githubusercontent.com/openai/codex/main/"
    "codex-rs/models-manager/models.json"
)
ASSET_PATH = Path(__file__).resolve().parent.parent / "assets" / "codex-models.json"

# 官方 visibility 为 hide 的条目不补入：它们不进官方模型选择器
# （内部预留位/后台模型/未发布实验），第三方中转也不会使用这些 slug。
SKIP_SLUGS = {"gpt-reserve", "codex-auto-review", "gpt-daybreak-blue-latest", "gpt-daybreak-red-latest"}


def fetch_official_models() -> dict:
    with urllib.request.urlopen(SOURCE_URL, timeout=60) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    models = data.get("models")
    if not isinstance(models, list):
        raise RuntimeError("官方 models.json 结构异常：缺少 models 数组")
    return {m["slug"]: m for m in models if isinstance(m, dict) and m.get("slug")}


def load_asset() -> list:
    with open(ASSET_PATH, encoding="utf-8") as f:
        data = json.load(f)
    models = data.get("models")
    if not isinstance(models, list):
        raise RuntimeError(f"{ASSET_PATH} 结构异常：缺少 models 数组")
    return models


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="只报告差异，不写入")
    args = parser.parse_args()

    official = fetch_official_models()
    asset_models = load_asset()
    existing = {m["slug"] for m in asset_models}

    additions = [
        slug for slug in official
        if slug not in existing and slug not in SKIP_SLUGS
    ]

    if not additions:
        print("静态资产已覆盖官方仓库全部可见模型，无需补入。")
        return 0

    for slug in additions:
        entry = dict(official[slug])
        # comp_hash：服务端一致性标记，语义未公开，不带入静态资产
        entry.pop("comp_hash", None)
        # 插在存量块之前，保持「官方现役在前、存量在后」的分区
        legacy_first = next(
            (i for i, m in enumerate(asset_models)
             if m["slug"] in ("gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex", "gpt-5.2")),
            len(asset_models),
        )
        asset_models.insert(legacy_first, entry)
        print(f"补入 {slug} (max_context_window={entry.get('max_context_window')})")

    if args.check:
        print(f"--check 模式：发现 {len(additions)} 条可补入，未写入。")
        return 1

    with open(ASSET_PATH, "w", encoding="utf-8", newline="\n") as f:
        json.dump({"models": asset_models}, f, ensure_ascii=False, indent=2)
        f.write("\n")
    print(f"已写入 {ASSET_PATH}（现 {len(asset_models)} 条）。")
    print("请运行 cargo test -p codex-plus-core --test model_suffix --test relay_config 验证后提交。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
