import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LogWindow } from "./LogView";
import "./styles.css";

// Set by the open_log_window command's initialisation script.
const logTarget = (window as { __PIPESCOPE_LOG__?: { key: string; step: string | null } }).__PIPESCOPE_LOG__;

const start = () =>
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      {logTarget ? <LogWindow runKey={logTarget.key} step={logTarget.step ?? undefined} /> : <App />}
    </React.StrictMode>,
  );

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  import("./devMock").then((m) => {
    m.install();
    start();
  });
} else {
  start();
}
