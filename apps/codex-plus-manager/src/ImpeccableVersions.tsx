import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";

type Versions = {
  status: string;
  message: string;
  skillVersion?: string | null;
  engineVersion?: string | null;
  requiredEngineVersion?: string | null;
};

export function ImpeccableVersions({ instanceKey }: { instanceKey: string }) {
  const [versions, setVersions] = useState<Versions | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    setVersions(null);
    setError("");
    void invoke<Versions>("inspect_impeccable_versions", { instanceKey }).then((result) => {
      if (!active) return;
      if (result.status === "ok") setVersions(result);
      else setError(result.message);
    }, (reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [instanceKey]);
  if (error) return <p className="mg-warning" role="alert">{error}</p>;
  if (!versions) return <p className="mg-muted" role="status">{t("正在读取版本…")}</p>;
  return <dl className="mg-detail">
    <dt>{t("技能版本")}</dt><dd>{versions.skillVersion || t("未记录")}</dd>
    <dt>{t("实际引擎版本")}</dt><dd>{versions.engineVersion || t("未知")}</dd>
    <dt>{t("启动器要求的引擎版本")}</dt><dd>{versions.requiredEngineVersion || t("未记录")}</dd>
  </dl>;
}
