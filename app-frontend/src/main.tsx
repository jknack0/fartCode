import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// Bundled typefaces (fontsource) — the Tauri webview must not depend on a
// CDN. emdash world: Inter Variable = the one UI voice, JetBrains Mono
// Variable = machine strings and terminals.
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./styles.css";
import { startWebviewWatchdog } from "./lib/webviewWatchdog";

// tauri#14843: self-heal the macOS webview-frame desync (black strip at the
// window bottom / clipped content) — see lib/webviewWatchdog.ts.
startWebviewWatchdog();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
