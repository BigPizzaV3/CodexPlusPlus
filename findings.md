# 整合核验

- 起始分支 codex/local-combined-2250，HEAD b2993b9。
- 初始未提交 App.tsx 和 i18n-en.ts：本地脚本更新计数和更新按钮，全部保留。
- 备份 C:/Users/lucy/Desktop/codex-combined-backup-20260920-035620。
- 5 个现有文件的会话修复补丁 git apply --check 通过后应用，无文本冲突；3 个新文件仅在目标不存在时添加。
- 未复制原分支整份 App.tsx/lib.rs，不携入无关 mobile-relay 和 AGENTS 改动。
