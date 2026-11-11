// ClaudeMom 的入口（claude-mom.html）。和 Codex++ 的 main.tsx 分开：自己的界面、自己的样式，只复用后端命令。
import "../legacy-webkit";
import { Component, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { ClaudeMomApp } from "./ClaudeMomApp";
import "./claude-mom.css";

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("[ClaudeMom] 界面渲染失败", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="cm-crash" role="alert">
        <h1>ClaudeMom 界面加载失败</h1>
        <pre>{error.stack || error.message}</pre>
        <button className="cm-btn" onClick={() => window.location.reload()} type="button">重新加载</button>
      </div>
    );
  }
}

// macOS 去掉了系统标题栏，红绿灯浮在侧栏左上角，侧栏顶部要留出位置。
if (/Mac/i.test(navigator.platform || navigator.userAgent)) document.documentElement.classList.add("cm-mac");

const root = document.getElementById("app");
if (root) {
  createRoot(root).render(
    <ErrorBoundary>
      <ClaudeMomApp />
    </ErrorBoundary>,
  );
}
