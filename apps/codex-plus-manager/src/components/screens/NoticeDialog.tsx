import { Bell, CheckCircle2 } from "lucide-react";
import { useEffect } from "react";
import { Status, TOAST_AUTO_CLOSE_MS } from "./model";
export function NoticeDialog({ notice, onClose, }: {
    notice: {
        title: string;
        message: string;
        status?: Status;
    };
    onClose: () => void;
}) {
    useEffect(() => {
        const timer = window.setTimeout(onClose, TOAST_AUTO_CLOSE_MS);
        return () => window.clearTimeout(timer);
    }, []);
    return (<div className="toast-wrap" role="status" aria-live="polite">
      <div className={`toast-card ${notice.status === "failed" ? "failed" : ""}`}>
        <div className="toast-progress"/>
        <div className="toast-icon">
          {notice.status === "failed" ? <Bell className="h-5 w-5"/> : <CheckCircle2 className="h-5 w-5"/>}
        </div>
        <div className="toast-body">
          <h2>{notice.title}</h2>
          <p>{notice.message}</p>
        </div>
        <button className="toast-close" onClick={onClose} type="button">×</button>
      </div>
    </div>);
}
