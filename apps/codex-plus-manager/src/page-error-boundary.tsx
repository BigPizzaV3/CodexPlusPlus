// 单页出错兜底：某一页渲染时抛错，只在这一页显示原因和「重试」，侧栏、其它页照常能用。
// 换页（resetKey 变了）自动清掉错误；最外层的 StartupErrorBoundary 只兜整个窗口起不来的情况。
import { Component, type ErrorInfo, type ReactNode } from "react";
import { t } from "./i18n";
import { CxCard, CxEmpty } from "./cx-layout";

type Props = { resetKey: string; children: ReactNode };
type State = { error: Error | null; resetKey: string };

export class PageErrorBoundary extends Component<Props, State> {
  state: State = { error: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    // 切到别的页：上一页的错误不跟过去。
    return props.resetKey !== state.resetKey ? { error: null, resetKey: props.resetKey } : null;
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("[Codex++] 页面渲染失败", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="page-error" role="alert">
        <CxCard>
          <CxEmpty
            detail={<code className="page-error-message">{error.message || String(error)}</code>}
            title={t("这一页出错了，其它页面不受影响")}
          >
            <button className="cx-secondary" onClick={() => this.setState({ error: null })} type="button">{t("重试")}</button>
            <button className="cx-secondary" onClick={() => void navigator.clipboard?.writeText(`${error.message}\n${error.stack ?? ""}`)} type="button">{t("复制详情")}</button>
          </CxEmpty>
        </CxCard>
      </div>
    );
  }
}
