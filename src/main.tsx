import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import App from "./App";
import ErrorBoundary from "./shared/components/ErrorBoundary";
import { installBrowserAcceleratorBlocker } from "./shared/lib/blockBrowserAccelerators";
import { applyAccent, mirroredAccent } from "./shared/lib/accent";

// Suppress WebView browser accelerators (reload, find…).
installBrowserAcceleratorBlocker();

// Last chosen 主题色, so the window does not flash blue before settings load.
applyAccent(mirroredAccent());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);
