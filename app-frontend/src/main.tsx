import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// Bundled typefaces (fontsource) — the Tauri webview must not depend on a
// CDN. Space Grotesk = UI voice, JetBrains Mono = data/terminal voice.
import "@fontsource/space-grotesk/400.css";
import "@fontsource/space-grotesk/500.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/space-grotesk/700.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
