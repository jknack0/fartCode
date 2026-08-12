// tauri#14843 watchdog: on macOS the WKWebView can get stuck painting short
// of the window (dead black strip at the bottom, or content clipped past the
// edge). The webview itself is the only place the desync is reliably
// observable — window.innerHeight is its BELIEF, the window's innerSize is
// the TRUTH — so compare the two and ask the shell for the native frame
// jiggle (`webview_resync`) when they disagree. Runs on resize plus a slow
// interval; a no-op when healthy, so it's safe everywhere.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const TOLERANCE_PX = 2;
const INTERVAL_MS = 2000;

let started = false;
let checking = false;

async function check(): Promise<void> {
  if (checking) return;
  checking = true;
  try {
    const win = getCurrentWindow();
    const [inner, scale] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    const expectedH = inner.height / scale;
    const expectedW = inner.width / scale;
    if (
      Math.abs(window.innerHeight - expectedH) > TOLERANCE_PX ||
      Math.abs(window.innerWidth - expectedW) > TOLERANCE_PX
    ) {
      console.warn(
        `webview desync: believed ${window.innerWidth}x${window.innerHeight}, window ${expectedW}x${expectedH} — resyncing`,
      );
      await invoke("webview_resync");
    }
  } catch {
    // Not running under Tauri (vitest/jsdom) or the window is gone — idle.
  } finally {
    checking = false;
  }
}

/** Idempotent; called once from main.tsx. */
export function startWebviewWatchdog(): void {
  if (started) return;
  started = true;
  window.addEventListener("resize", () => void check());
  setInterval(() => void check(), INTERVAL_MS);
}
