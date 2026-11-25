import { Component, type ErrorInfo, type ReactNode } from "react";
import { formatError } from "../lib/errorMessage";

// Rendered above App, so translations are not loaded yet; the main window
// mirrors the chosen language into localStorage for exactly this purpose.
const TEXT = {
  zh: { title: "界面异常", body: "复制记录已保留。\n重新加载后可继续使用。", reload: "重新加载" },
  en: { title: "Something went wrong", body: "Your clipboard history is safe. Reload the window to continue.", reload: "Reload" },
} as const;

const pickText = () => {
  return localStorage.getItem("copy_paste_language") === "en" ? TEXT.en : TEXT.zh;
};

interface State {
  error: unknown;
}

/** Keeps a render error in one component from blanking the whole window. */
export default class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: unknown): State {
    return { error };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("UI crashed:", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    const text = pickText();
    return (
      // The app's own empty-state layout and primary button: the error page
      // used class names no stylesheet defines.
      <div className="app-shell">
        <div className="empty-state error-boundary" role="alert">
          <p className="empty-title">{text.title}</p>
          <p className="empty-hint error-boundary-body">{text.body}</p>
          <p className="empty-hint error-boundary-detail">{formatError(this.state.error, undefined, 160)}</p>
          <button type="button" className="glass-button is-primary" onClick={() => window.location.reload()}>
            {text.reload}
          </button>
        </div>
      </div>
    );
  }
}
