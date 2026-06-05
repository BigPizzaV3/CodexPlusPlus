import { Card, CardDescription, CardHeader, CardTitle } from "../ui/card";
import { Label } from "../ui/label";
export function Panel({ children, fill = false, className = "" }: {
    children: React.ReactNode;
    fill?: boolean;
    className?: string;
}) {
    return (<Card className={`panel ${fill ? "fill" : ""} ${className}`}>
      {children}
    </Card>);
}
export function CardHead({ title, detail }: {
    title: string;
    detail: string;
}) {
    return (<CardHeader className="panel-head">
      <CardTitle>{title}</CardTitle>
      <CardDescription>{detail}</CardDescription>
    </CardHeader>);
}
export function Toolbar({ children }: {
    children: React.ReactNode;
}) {
    return <div className="toolbar">{children}</div>;
}
export function Field({ label, children, className = "" }: {
    label: string;
    children: React.ReactNode;
    className?: string;
}) {
    return (<Label className={`field ${className}`}>
      <span>{label}</span>
      {children}
    </Label>);
}
