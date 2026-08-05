// Interactive terminal sessions (E2-12): xterm instances live OUTSIDE React,
// keyed by PTY id, so task switches and tab flips detach/reattach the same
// shell + scrollback instead of recreating either. PTY lifetime is owned by
// the tab store (closeTab / dropTask call killTerminal / disposeTerminalSession)
// — component cleanup only detaches the DOM, never kills the shell.
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  onTerminalExited,
  onTerminalOutput,
  terminalClose,
  terminalWrite,
} from "./tauri";

export interface TermSession {
  term: Terminal;
  fit: FitAddon;
  /** Detached/reattached by TerminalView; owns the xterm DOM. */
  host: HTMLDivElement;
  exited: boolean;
  dispose: () => void;
}

const sessions = new Map<string, TermSession>();

export function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Lazily builds (or returns) the live session for a PTY id. */
export function getTerminalSession(terminalId: string): TermSession {
  const existing = sessions.get(terminalId);
  if (existing) return existing;

  const term = new Terminal({
    cursorBlink: true,
    // Nerd Fonts first so Powerline/nerd glyphs (, , icons) render;
    // machines without them fall back to the system mono stack.
    fontFamily:
      '"JetBrainsMono Nerd Font Mono", "MesloLGS Nerd Font Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
    fontSize: 12,
    theme: {
      background: "#0d1424",
      foreground: "#e5e7eb",
      cursor: "#d97706",
      selectionBackground: "rgba(217, 119, 6, 0.3)",
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const host = document.createElement("div");
  host.className = "terminal-surface";
  term.open(host);

  let disposed = false;
  let exited = false;
  const unsubs: Array<() => void> = [];
  void onTerminalOutput(({ terminalId: id, data }) => {
    if (id === terminalId && !disposed && !exited) {
      term.write(base64ToBytes(data));
    }
  }).then((un) => unsubs.push(un));
  void onTerminalExited(({ terminalId: id, exitCode }) => {
    if (id === terminalId && !disposed) {
      exited = true;
      session.exited = true;
      term.write(
        `\r\n[process exited${exitCode !== null ? ` with code ${exitCode}` : ""}] — close this tab\r\n`,
      );
    }
  }).then((un) => unsubs.push(un));

  const onData = term.onData((data) => {
    if (!exited) void terminalWrite(terminalId, data).catch(() => {});
  });

  const session: TermSession = {
    term,
    fit,
    host,
    exited: false,
    dispose: () => {
      if (disposed) return;
      disposed = true;
      onData.dispose();
      unsubs.forEach((un) => un());
      term.dispose();
      sessions.delete(terminalId);
    },
  };
  sessions.set(terminalId, session);
  return session;
}

/** Frees the xterm session without touching the backend PTY (task delete
 * teardown — the backend's close_task already killed those shells). */
export function disposeTerminalSession(terminalId: string): void {
  sessions.get(terminalId)?.dispose();
}

/** Kills the shell behind a terminal tab and discards its session. */
export function killTerminal(terminalId: string): void {
  disposeTerminalSession(terminalId);
  // Errors are fine — the PTY may already be gone (shell exited on its own).
  void terminalClose(terminalId).catch(() => {});
}
