import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus } from "lucide-react";
import { Action } from "./ManagementPages";
import { t } from "./i18n";
import { parseSkillRepository, type SkillRepository } from "./skill-repository";

export function SkillRepositoryQuickAdd({ repos, disabled, onBusyChange, onAdded }: {
  repos: SkillRepository[];
  disabled: boolean;
  onBusyChange: (busy: boolean) => void;
  onAdded: (repos: SkillRepository[]) => void;
}) {
  const [input, setInput] = useState("");
  const [branch, setBranch] = useState("main");
  const [subdir, setSubdir] = useState("");
  const [feedback, setFeedback] = useState("");
  const pending = useRef(false);
  const add = async () => {
    if (disabled || pending.current) return;
    const parsed = parseSkillRepository(input);
    if (!parsed) {
      setFeedback(t("无法识别仓库地址，请输入 GitHub 链接或 owner/repo。"));
      return;
    }
    const repo = { ...parsed, branch: branch.trim() || "main", subdir: subdir.trim().replace(/^\/+|\/+$/g, "") };
    if (repo.subdir && repo.subdir.split("/").some((part) => !part || part === "." || part === ".." || part.includes("\\"))) {
      setFeedback(t("子目录格式无效。"));
      return;
    }
    if (repos.some((item) => item.owner.toLowerCase() === repo.owner.toLowerCase() && item.name.toLowerCase() === repo.name.toLowerCase()
      && item.branch === repo.branch && item.subdir === repo.subdir)) {
      setFeedback(t("该仓库已添加，请直接选择仓库路径。"));
      return;
    }
    pending.current = true;
    onBusyChange(true);
    setFeedback("");
    try {
      const result = await invoke<{ status: string; message: string; repos: SkillRepository[] }>("add_skill_repo", { repo });
      setFeedback(t(result.message));
      if (result.status === "ok") {
        setInput("");
        setBranch("main");
        setSubdir("");
        onAdded(result.repos);
      }
    } catch (error) {
      setFeedback(String(error));
    } finally {
      pending.current = false;
      onBusyChange(false);
    }
  };
  return <div className="mg-source-match-add">
    <div className="mg-repo-quick">
      <input aria-label={t("GitHub 仓库地址")} placeholder="https://github.com/owner/repo" value={input}
        disabled={disabled} onChange={(event) => {
          const value = event.currentTarget.value;
          setInput(value); setFeedback("");
          const parsed = parseSkillRepository(value);
          if (parsed) { setBranch(parsed.branch); setSubdir(parsed.subdir); }
        }} />
      <Action disabled={disabled || !input.trim()} onClick={() => void add()}><Plus size={14}/>{t("验证并添加仓库")}</Action>
    </div>
    <div className="mg-source-match-fields">
      <label>{t("分支")}<input aria-label={t("分支")} value={branch} disabled={disabled} onChange={(event) => setBranch(event.currentTarget.value)}/></label>
      <label>{t("子目录")}<input aria-label={t("子目录")} value={subdir} disabled={disabled} onChange={(event) => setSubdir(event.currentTarget.value)}/></label>
    </div>
    {feedback && <small role="status">{feedback}</small>}
  </div>;
}
