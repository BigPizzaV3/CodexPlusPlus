import { Badge } from "./statusWidgets";
export function FeatureItem({ title, detail, enabled }: {
    title: string;
    detail: string;
    enabled: boolean;
}) {
    return (<div className="feature-item">
      <div>
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
      <Badge status={enabled ? "ok" : "disabled"}/>
    </div>);
}
export function FeatureToggle({ title, detail, checked, disabled = false, onChange, }: {
    title: string;
    detail: string;
    checked: boolean;
    disabled?: boolean;
    onChange: (value: boolean) => void;
}) {
    return (<label className={`feature-toggle ${disabled ? "disabled" : ""}`}>
      <input checked={checked} disabled={disabled} onChange={(event) => onChange(event.currentTarget.checked)} type="checkbox"/>
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
      <Badge status={!disabled && checked ? "ok" : "disabled"}/>
    </label>);
}
export function GuideList({ items }: {
    items: string[];
}) {
    return (<div className="guide-list">
      {items.map((item, index) => (<div className="guide-step" key={item}>
          <span>{index + 1}</span>
          <p>{item}</p>
        </div>))}
    </div>);
}
