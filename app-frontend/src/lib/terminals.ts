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
  terminalTail,
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
    // Nerd Fonts first so Powerline/nerd glyphs render; then the bundled
    // JetBrains Mono; last resort the system mono stack.
    fontFamily:
      '"JetBrainsMono Nerd Font Mono", "MesloLGS Nerd Font Mono", "JetBrains Mono Variable", ui-monospace, SFMono-Regular, Menlo, monospace',
    fontSize: 12,
    // Matches the emdash world in styles.css — keep in sync with
    // --xterm-bg / --xterm-fg / --xterm-selection-*.
    theme: {
      background: "#181818",
      foreground: "#e9e8e8",
      cursor: "#e9e8e8",
      selectionBackground: "rgba(57, 142, 255, 0.475)",
      selectionForeground: "#82baff",
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const host = document.createElement("div");
  host.className = "terminal-surface";
  term.open(host);

  let disposed = false;
  let exited = false;
  let ready = false;
  const pending: Uint8Array[] = [];
  const unsubs: Array<() => void> = [];
  // Subscribe BEFORE fetching the scrollback tail so output arriving in
  // between is buffered, not lost; the tail then replays first (reattach
  // after a frontend reload — plain PTYs have no scrollback server).
  void (async () => {
    const un = await onTerminalOutput(({ terminalId: id, data }) => {
      if (id !== terminalId || disposed || exited) return;
      const bytes = base64ToBytes(data);
      if (ready) term.write(bytes);
      else pending.push(bytes);
    });
    unsubs.push(un);
    const tail = await terminalTail(terminalId).catch(() => null);
    if (tail && !disposed) term.write(base64ToBytes(tail));
    ready = true;
    for (const bytes of pending) term.write(bytes);
    pending.length = 0;
  })();
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
