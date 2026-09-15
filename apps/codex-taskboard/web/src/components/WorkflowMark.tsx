import { LinearIcon, type LinearIconName } from "./LinearIcon";

interface WorkflowMarkProps {
  icon: LinearIconName;
  logo?: string;
  logoMonochrome?: boolean;
}

export function WorkflowMark({ icon, logo, logoMonochrome = false }: WorkflowMarkProps) {
  if (!logo) return <LinearIcon name={icon} />;

  return (
    <img
      className={`workflow-brand-logo${logoMonochrome ? " monochrome" : ""}`}
      src={logo}
      alt=""
      draggable={false}
    />
  );
}
