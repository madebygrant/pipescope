import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LogWindow } from "./LogView";
import "@fontsource/inter-tight/latin-400.css";
import "@fontsource/inter-tight/latin-500.css";
import "@fontsource/inter-tight/latin-600.css";
import "@fontsource/inter-tight/latin-700.css";
import "@fontsource/inter-tight/latin-800.css";
import "@fontsource/inter-tight/latin-900.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "@fontsource/jetbrains-mono/latin-700.css";
import "./styles.css";
import { followTheme } from "./theme";

// Set by the open_log_window command's initialisation script.
const logTarget = (window as { __PIPESCOPE_LOG__?: { key: string; step: string | null } }).__PIPESCOPE_LOG__;

followTheme();

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
