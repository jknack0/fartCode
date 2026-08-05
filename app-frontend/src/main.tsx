import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// Bundled typefaces (fontsource) — the Tauri webview must not depend on a
// CDN. emdash world: Inter Variable = the one UI voice, JetBrains Mono
// Variable = machine strings and terminals.
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
